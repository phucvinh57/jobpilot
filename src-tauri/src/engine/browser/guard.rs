//! The request guard of the browsers that read third-party pages for job discovery.
//!
//! A page from the internet must not be able to make the browser reach this machine or the local
//! network (its document, a redirect, an image, a script, a frame, a worker, an `fetch()` that posts
//! the answer somewhere else). Checking requests inside the browser would miss some of those (frames
//! and workers live in other targets) and would resolve names twice (so a name could answer with a
//! public address first and a private one later). So the browser is pointed at a small **forward
//! proxy on this machine** ([`Guard`]) and every connection it makes goes through it:
//!
//! - `CONNECT host:port` (https, and anything else tunnelled) and `GET http://host/...` (plain http)
//!   are the only requests served;
//! - the host gets the rules of [`net::http`](crate::engine::net::http): not a machine-local or private name or
//!   address, and a name is resolved by that module's resolver (private answers dropped);
//! - the proxy connects to the address it resolved itself, so what was checked is what is used;
//! - a refused request is answered `403` (a tunnel) or just dropped (plain http) and counted
//!   ([`Guard::blocked`]), and nothing reaches the private host.
//!
//! The proxy only relays bytes (a TLS tunnel stays end to end, to the one host that was checked); a
//! plain http request is sent on in origin form with `Connection: close`, and a plain connection
//! carries **one request**: its body is relayed by its `Content-Length`, the answer is relayed, the
//! connection is closed, and nothing else the browser wrote on it (a pipelined request for another
//! host) is ever sent anywhere. It listens on `127.0.0.1` on a free port, for one browser, and
//! stops when the [`Guard`] is dropped. The browser must be started with [`Guard::chrome_args`], which
//! also turns off QUIC and non-proxied WebRTC (UDP would go around a proxy) and removes Chrome's
//! implicit "loopback is never proxied" rule.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

use crate::engine::net::http::{ip_is_public, is_public_host, public_socket_addrs, NotPublic};

/// Longest request head accepted.
const HEAD_MAX: usize = 16 * 1024;
/// Largest body of a plain http request that is sent on.
const BODY_MAX: usize = 16 * 1024 * 1024;
/// How long the browser has to send a request head, and the proxy to connect.
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

/// Which hosts the guard lets the browser reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Public hosts only (the rules of `net::http`).
    Public,
    /// Everything (tests that read fixture pages on `127.0.0.1`).
    Anything,
    /// Tests: these local ports stand for public hosts, every other address is private.
    #[cfg(test)]
    OnlyPorts(Vec<u16>),
}

impl Policy {
    /// `Anything` when `allow_private`, else `Public`.
    pub(crate) fn new(allow_private: bool) -> Self {
        if allow_private {
            Policy::Anything
        } else {
            Policy::Public
        }
    }

    /// May an address on this machine or a private network be *asked for* (the first check of a
    /// page address, before the browser starts)?
    pub(crate) fn allows_private_addresses(&self) -> bool {
        !matches!(self, Policy::Public)
    }
}

/// A running guard proxy. Dropping it stops the proxy and every connection through it.
pub(crate) struct Guard {
    addr: SocketAddr,
    blocked: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Guard {
    /// Start a proxy on a free local port that lets the browser reach what `policy` says.
    pub(crate) async fn start(policy: Policy) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await.context("starting the request guard")?;
        let addr = listener.local_addr().context("the request guard has no address")?;
        let blocked = Arc::new(AtomicUsize::new(0));
        let count = blocked.clone();
        let task = tokio::spawn(async move {
            // Dropping the set (the task is aborted) ends the connections too.
            let mut conns = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => {
                            let (count, policy) = (count.clone(), policy.clone());
                            conns.spawn(async move {
                                if let Err(e) = serve(stream, &policy, &count).await {
                                    tracing::debug!("request guard: {e:#}");
                                }
                            });
                        }
                        Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                    },
                    Some(_) = conns.join_next(), if !conns.is_empty() => {}
                }
            }
        });
        Ok(Self { addr, blocked, task })
    }

    /// `http://127.0.0.1:<port>`
    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The Chrome flags that send every request through this proxy.
    pub(crate) fn chrome_args(&self) -> Vec<String> {
        vec![
            format!("--proxy-server={}", self.url()),
            // Chrome never proxies localhost and loopback addresses unless told so.
            "--proxy-bypass-list=<-loopback>".to_string(),
            "--disable-quic".to_string(),
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_string(),
        ]
    }

    /// How many requests were refused so far.
    pub(crate) fn blocked(&self) -> usize {
        self.blocked.load(Ordering::SeqCst)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Why a request was not served.
enum Refusal {
    /// The host is private, local or only resolves to such addresses.
    Private,
    /// The host is public but could not be reached (or the request is malformed).
    Failed(anyhow::Error),
}

/// What the browser asked for.
enum Request {
    /// A tunnel to `https://host:port` (the URL is only a parsed host and port).
    Tunnel(url::Url),
    /// A plain http request: where it goes, the head to send there and how long its body is.
    Plain { url: url::Url, head: Vec<u8>, body_len: usize },
}

async fn serve(mut client: TcpStream, policy: &Policy, blocked: &AtomicUsize) -> Result<()> {
    let (head, rest) = tokio::time::timeout(STEP_TIMEOUT, read_head(&mut client)).await.context("no request head")??;
    let request = parse(&head)?;
    let url = match &request {
        Request::Tunnel(url) | Request::Plain { url, .. } => url,
    };
    let mut upstream = match connect(url, policy).await {
        Ok(upstream) => upstream,
        Err(Refusal::Private) => {
            blocked.fetch_add(1, Ordering::SeqCst);
            tracing::info!("request guard: a request to a private address was refused");
            if matches!(request, Request::Tunnel(_)) {
                let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            }
            return Ok(());
        }
        Err(Refusal::Failed(e)) => {
            if matches!(request, Request::Tunnel(_)) {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            }
            return Err(e);
        }
    };
    match request {
        Request::Tunnel(_) => {
            client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
            upstream.write_all(&rest).await?;
            tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
        }
        Request::Plain { head, body_len, .. } => {
            upstream.write_all(&head).await?;
            // **One request per connection.** Only this request's own body goes on; whatever else the
            // browser sent on the socket (a pipelined second request, one sent after the answer) is
            // dropped, because it could name a host that was never checked.
            let from_rest = rest.len().min(body_len);
            upstream.write_all(&rest[..from_rest]).await?;
            let mut body = (&mut client).take((body_len - from_rest) as u64);
            tokio::io::copy(&mut body, &mut upstream).await?;
            // The answer (the server was told `Connection: close`), then the connection ends.
            tokio::io::copy(&mut upstream, &mut client).await?;
            let _ = client.shutdown().await;
        }
    }
    Ok(())
}

/// Read up to the end of a request head. Returns the head (with its blank line) and whatever came
/// after it in the same reads.
async fn read_head(client: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = raw.split_off(at + 4);
            return Ok((raw, rest));
        }
        if raw.len() > HEAD_MAX {
            bail!("the request head is too long");
        }
        match client.read(&mut buf).await? {
            0 => bail!("the browser closed the connection before sending a request"),
            n => raw.extend_from_slice(&buf[..n]),
        }
    }
}

/// Hop-by-hop headers a plain request does not pass on.
const HOP_HEADERS: [&str; 6] = ["proxy-connection", "connection", "keep-alive", "proxy-authorization", "upgrade", "te"];

fn parse(head: &[u8]) -> Result<Request> {
    let mut lines = head.split(|b| *b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    let first = std::str::from_utf8(lines.next().unwrap_or_default()).map_err(|_| anyhow!("the request line is not text"))?;
    let mut parts = first.split(' ');
    let (method, target, version) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v), None) if v.starts_with("HTTP/1.") && !m.is_empty() => (m, t, v),
        _ => bail!("not an HTTP proxy request"),
    };
    if method.eq_ignore_ascii_case("CONNECT") {
        // `host:port` and nothing else (no userinfo, path or query to smuggle another host in).
        if target.is_empty() || target.contains(['/', '@', '?', '#', '\\', ' ']) {
            bail!("a tunnel target must be host:port");
        }
        let url = url::Url::parse(&format!("https://{target}")).map_err(|_| anyhow!("a tunnel target must be host:port"))?;
        if url.host().is_none() {
            bail!("a tunnel target must be host:port");
        }
        return Ok(Request::Tunnel(url));
    }
    let url = url::Url::parse(target).map_err(|_| anyhow!("a plain request must name its address in full"))?;
    if url.scheme() != "http" || url.host().is_none() || !url.username().is_empty() {
        bail!("only http addresses are sent plain");
    }
    let origin = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let mut out = format!("{method} {origin} {version}\r\n").into_bytes();
    let mut body_len = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = match line.iter().position(|b| *b == b':') {
            Some(at) => (&line[..at], &line[at + 1..]),
            None => (line, &line[line.len()..]),
        };
        if HOP_HEADERS.iter().any(|h| name.eq_ignore_ascii_case(h.as_bytes())) {
            continue;
        }
        // The body has to be framed by a length, so that it is known where this request ends.
        if name.eq_ignore_ascii_case(b"transfer-encoding") {
            bail!("a body without a length is not sent through the guard");
        }
        if name.eq_ignore_ascii_case(b"content-length") {
            let len = std::str::from_utf8(value).ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| *n <= BODY_MAX);
            match (len, body_len) {
                (Some(len), None) => body_len = Some(len),
                _ => bail!("the request has a bad or repeated Content-Length"),
            }
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    Ok(Request::Plain { url, head: out, body_len: body_len.unwrap_or(0) })
}

/// Connect to `url`'s host and port: to an address `policy` allows (by default the ones the net
/// rules allow).
async fn connect(url: &url::Url, policy: &Policy) -> Result<TcpStream, Refusal> {
    let addrs = tokio::time::timeout(STEP_TIMEOUT, resolve(url, policy))
        .await
        .map_err(|_| Refusal::Failed(anyhow!("resolving the name: timed out")))??;
    let mut last = anyhow!("the host has no address");
    for addr in addrs {
        match tokio::time::timeout(STEP_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last = anyhow!("connecting: {e}"),
            Err(_) => last = anyhow!("connecting: timed out"),
        }
    }
    Err(Refusal::Failed(last))
}

/// The addresses `policy` lets `url` be reached at. A host or name that is not public is
/// [`Refusal::Private`]; a name that does not resolve is a plain failure.
async fn resolve(url: &url::Url, policy: &Policy) -> Result<Vec<SocketAddr>, Refusal> {
    match policy {
        Policy::Anything => any_addrs(url).await.map_err(Refusal::Failed),
        Policy::Public => public_socket_addrs(url).await.map_err(|e| if e.is::<NotPublic>() { Refusal::Private } else { Refusal::Failed(e) }),
        #[cfg(test)]
        Policy::OnlyPorts(ports) => match url.port_or_known_default() {
            Some(port) if ports.contains(&port) => any_addrs(url).await.map_err(Refusal::Failed),
            _ => Err(Refusal::Private),
        },
    }
}

/// How long resolving a name may take for [`allows`].
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

/// May a guarded browser tab request `url` (the browser bridge's guard, which answers each request
/// the tab is about to make)? Only http(s) and ws(s) addresses are judged; other schemes (`data:`,
/// `blob:`, `chrome-extension:`, ...) reach no host and are allowed. Under [`Policy::Public`] the
/// host must be public by the `net::http` rules and, for a name, **every** address it resolves to
/// must be public (the browser resolves it again and may pick any of them). A name that does not
/// resolve is refused.
pub(crate) async fn allows(policy: &Policy, url: &url::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return true;
    }
    match policy {
        Policy::Anything => true,
        Policy::Public => {
            if !is_public_host(url) {
                return false;
            }
            match url.host() {
                Some(url::Host::Domain(name)) => {
                    let lookup = tokio::net::lookup_host((name.trim_end_matches('.'), 0));
                    match tokio::time::timeout(RESOLVE_TIMEOUT, lookup).await {
                        Ok(Ok(addrs)) => {
                            let addrs: Vec<_> = addrs.collect();
                            !addrs.is_empty() && addrs.iter().all(|a| ip_is_public(a.ip()))
                        }
                        _ => false,
                    }
                }
                // An address literal: `is_public_host` judged it.
                Some(_) => true,
                None => false,
            }
        }
        #[cfg(test)]
        Policy::OnlyPorts(ports) => url.port_or_known_default().is_some_and(|port| ports.contains(&port)),
    }
}

/// Every address of `url`'s host, private or not.
async fn any_addrs(url: &url::Url) -> Result<Vec<SocketAddr>> {
    let port = url.port_or_known_default().unwrap_or(80);
    Ok(match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(url::Host::Domain(name)) => tokio::net::lookup_host((name, port)).await?.collect(),
        None => bail!("the address has no host"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::net::fixtures::Fixtures;

    /// Send `raw` to the proxy and read until it closes the connection.
    async fn ask(guard: &Guard, raw: &str) -> String {
        let mut stream = TcpStream::connect(guard.addr).await.unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out)).await;
        String::from_utf8_lossy(&out).into_owned()
    }

    fn port_of(fx: &Fixtures) -> u16 {
        url::Url::parse(fx.base()).unwrap().port().unwrap()
    }

    #[tokio::test]
    async fn requests_to_private_and_local_hosts_are_refused_and_counted_and_never_sent() {
        let fx = Fixtures::start().await;
        fx.route("/secret", 200, "text/plain", "the admin console");
        let port = port_of(&fx);
        let guard = Guard::start(Policy::Public).await.unwrap();
        assert!(guard.url().starts_with("http://127.0.0.1:"));

        let mut refused = 0;
        for target in [
            format!("127.0.0.1:{port}"),
            "[::1]:443".to_string(),
            "localhost:80".to_string(),
            "10.0.0.5:443".to_string(),
            "192.168.1.1:8080".to_string(),
            "169.254.169.254:80".to_string(),
            "intranet:443".to_string(),
            "printer.local:631".to_string(),
        ] {
            let answer = ask(&guard, &format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n")).await;
            assert!(answer.starts_with("HTTP/1.1 403"), "{target}: {answer}");
            refused += 1;
        }
        // Plain http is dropped without an answer, so the browser shows a network error.
        let answer = ask(&guard, &format!("GET http://127.0.0.1:{port}/secret HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).await;
        assert_eq!(answer, "");
        refused += 1;
        assert_eq!(guard.blocked(), refused);
        assert!(fx.hits().is_empty(), "nothing reached the local server: {:?}", fx.hits());
    }

    #[tokio::test]
    async fn malformed_and_unsupported_requests_are_dropped_without_being_counted_as_private() {
        let guard = Guard::start(Policy::Anything).await.unwrap();
        for raw in [
            "GET /x HTTP/1.1\r\nHost: a.test\r\n\r\n",
            "GET https://a.test/ HTTP/1.1\r\n\r\n",
            "GET ftp://a.test/ HTTP/1.1\r\n\r\n",
            "GET http://user:pw@a.test/ HTTP/1.1\r\n\r\n",
            "CONNECT a.test:443@127.0.0.1:80 HTTP/1.1\r\n\r\n",
            "CONNECT a.test/x:443 HTTP/1.1\r\n\r\n",
            "CONNECT HTTP/1.1\r\n\r\n",
            "garbage\r\n\r\n",
        ] {
            assert_eq!(ask(&guard, raw).await, "", "{raw:?}");
        }
        assert_eq!(guard.blocked(), 0);
    }

    #[tokio::test]
    async fn a_guard_that_allows_private_hosts_relays_plain_http_and_tunnels() {
        let fx = Fixtures::start().await;
        fx.route("/hello", 200, "text/plain", "hello from the fixture");
        let (port, guard) = (port_of(&fx), Guard::start(Policy::Anything).await.unwrap());

        // Plain http: sent on in origin form, hop-by-hop headers dropped, `Connection: close` added.
        let answer = ask(
            &guard,
            &format!("GET http://127.0.0.1:{port}/hello?x=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nProxy-Connection: keep-alive\r\nAccept: text/html\r\n\r\n"),
        )
        .await;
        assert!(answer.starts_with("HTTP/1.1 200") && answer.ends_with("hello from the fixture"), "{answer}");
        let hit = &fx.hits_on("/hello")[0];
        assert_eq!((hit.method.as_str(), hit.target.as_str()), ("GET", "/hello?x=1"));
        assert_eq!((hit.header("connection"), hit.header("accept"), hit.header("proxy-connection")), (Some("close"), Some("text/html"), None));

        // A tunnel: after `200 Connection Established` the bytes go through untouched.
        let tunnel = ask(&guard, &format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: x\r\n\r\nGET /hello HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")).await;
        assert!(tunnel.starts_with("HTTP/1.1 200 Connection Established\r\n\r\nHTTP/1.1 200"), "{tunnel}");
        assert!(tunnel.ends_with("hello from the fixture"), "{tunnel}");
        assert_eq!(guard.blocked(), 0);

        // A closed port is a 502 for a tunnel, not a "private" refusal.
        let dead = ask(&guard, "CONNECT 127.0.0.1:1 HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(dead.starts_with("HTTP/1.1 502"), "{dead}");
        assert_eq!(guard.blocked(), 0);
    }

    /// Send `raw` pieces to the proxy (a pause between them) and read until it closes.
    async fn ask_in_pieces(guard: &Guard, pieces: &[&str]) -> String {
        let mut stream = TcpStream::connect(guard.addr).await.unwrap();
        for piece in pieces {
            let _ = stream.write_all(piece.as_bytes()).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out)).await;
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn a_plain_connection_carries_one_request_and_nothing_else_is_sent_on() {
        // `public` stands for a checked host, `private` for one the guard would refuse.
        let (public, private) = (Fixtures::start().await, Fixtures::start().await);
        public.route("/hello", 200, "text/plain", "hello").route("/post", 200, "text/plain", "posted");
        private.route("/secret", 200, "text/plain", "the admin console");
        let (a, b) = (port_of(&public), port_of(&private));
        let guard = Guard::start(Policy::OnlyPorts(vec![a])).await.unwrap();
        let get = |port: u16, path: &str| format!("GET http://127.0.0.1:{port}{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");

        // Pipelined: a second request for the private host in the same write is dropped.
        let answer = ask(&guard, &format!("{}{}", get(a, "/hello"), get(b, "/secret"))).await;
        assert!(answer.starts_with("HTTP/1.1 200") && answer.ends_with("hello"), "{answer}");
        assert_eq!(answer.matches("HTTP/1.1").count(), 1, "one answer: {answer}");
        // Sent after the answer on the same socket: not sent anywhere either.
        let late = ask_in_pieces(&guard, &[&get(a, "/hello"), &get(b, "/secret")]).await;
        assert_eq!(late.matches("HTTP/1.1").count(), 1, "{late}");
        assert_eq!(public.hits().len(), 2, "each connection asked its one request: {:?}", public.hits());
        assert!(public.hits().iter().all(|h| h.target == "/hello"), "nothing else reached the checked host: {:?}", public.hits());
        assert!(private.hits().is_empty(), "the unchecked host saw {:?}", private.hits());
        assert_eq!(guard.blocked(), 0, "the dropped request never got as far as being judged");

        // A body goes on by its length, and only that much.
        let post = format!("POST http://127.0.0.1:{a}/post HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello");
        let answer = ask_in_pieces(&guard, &[&post[..post.len() - 3], &format!("{}{}", &post[post.len() - 3..], get(b, "/secret"))]).await;
        assert!(answer.starts_with("HTTP/1.1 200") && answer.ends_with("posted"), "{answer}");
        let posted = &public.hits_on("/post")[0];
        assert_eq!((posted.method.as_str(), posted.body.as_str()), ("POST", "hello"), "the body, no more and no less");
        assert!(private.hits().is_empty(), "{:?}", private.hits());

        // A body without a length, or a bad one, is not sent (the end of the request would be unknown).
        let before = public.hits().len();
        for head in ["Transfer-Encoding: chunked", "Content-Length: abc", "Content-Length: 3\r\nContent-Length: 3", "Content-Length: 99999999999"] {
            let raw = format!("POST http://127.0.0.1:{a}/post HTTP/1.1\r\nHost: x\r\n{head}\r\n\r\n3\r\nabc\r\n0\r\n\r\n");
            assert_eq!(ask(&guard, &raw).await, "", "{head}");
        }
        assert_eq!(public.hits().len(), before);
    }

    #[tokio::test]
    async fn a_name_that_does_not_resolve_is_a_failure_not_a_private_refusal() {
        let guard = Guard::start(Policy::Public).await.unwrap();
        let answer = ask(&guard, "CONNECT does-not-exist.invalid:443 HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(answer.starts_with("HTTP/1.1 502"), "{answer}");
        assert_eq!(guard.blocked(), 0);
        // Private names stay refusals.
        let answer = ask(&guard, "CONNECT localhost:443 HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
        assert_eq!(guard.blocked(), 1);
    }

    #[tokio::test]
    async fn dropping_the_guard_stops_the_proxy() {
        let guard = Guard::start(Policy::Public).await.unwrap();
        let addr = guard.addr;
        drop(guard);
        let mut stopped = false;
        for _ in 0..50 {
            if TcpStream::connect(addr).await.is_err() {
                stopped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(stopped, "still listening");
    }

    #[tokio::test]
    async fn allows_judges_hosts_by_the_net_rules_and_lets_other_schemes_through() {
        let at = |raw: &str| url::Url::parse(raw).unwrap();
        for private in ["http://127.0.0.1/", "https://10.0.0.5/x", "http://[::1]:8080/", "http://localhost/", "http://intranet/", "https://printer.local/", "ws://192.168.1.1/", "http://169.254.169.254/"] {
            assert!(!allows(&Policy::Public, &at(private)).await, "{private}");
            assert!(allows(&Policy::Anything, &at(private)).await, "{private}");
        }
        assert!(allows(&Policy::Public, &at("https://93.184.215.14/")).await, "a public address literal");
        assert!(!allows(&Policy::Public, &at("https://does-not-exist.invalid/")).await, "a name that does not resolve");
        for other in ["data:text/html,hi", "blob:https://a.test/0b2c", "chrome-extension://abc/x.js", "about:blank"] {
            assert!(allows(&Policy::Public, &at(other)).await, "{other}");
        }
        let only = Policy::OnlyPorts(vec![4000]);
        assert!(allows(&only, &at("http://127.0.0.1:4000/")).await);
        assert!(!allows(&only, &at("http://127.0.0.1:4001/")).await);
        assert!(!allows(&only, &at("https://example.com/")).await);
    }

    #[test]
    fn the_chrome_flags_proxy_everything_and_close_the_udp_side_doors() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let args = rt.block_on(async { Guard::start(Policy::Public).await.unwrap().chrome_args() });
        assert!(args[0].starts_with("--proxy-server=http://127.0.0.1:"));
        for flag in ["--proxy-bypass-list=<-loopback>", "--disable-quic", "--force-webrtc-ip-handling-policy=disable_non_proxied_udp"] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
    }
}
