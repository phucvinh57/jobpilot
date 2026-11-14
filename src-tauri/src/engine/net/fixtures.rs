//! A fixture HTTP server on localhost for tests: routes by path, records every request, optional
//! per-route delay and redirects. Nothing leaves the machine.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
struct Route {
    path: String,
    status: u16,
    content_type: String,
    body: Vec<u8>,
    delay: Duration,
    /// `Location` header of a redirect.
    location: Option<String>,
}

/// One request the server saw.
#[derive(Clone, Debug)]
pub struct Hit {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Hit {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone)]
pub struct Fixtures {
    base: String,
    routes: Arc<Mutex<Vec<Route>>>,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Fixtures {
    /// Start serving on a free port. Add routes with [`route`](Self::route) at any time.
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind a local port");
        let base = format!("http://{}", listener.local_addr().expect("local address"));
        let this = Self { base, routes: Arc::default(), hits: Arc::default() };
        let server = this.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else { return };
                tokio::spawn(server.clone().serve(sock));
            }
        });
        this
    }

    /// `http://127.0.0.1:<port>`
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Answer `path` with `status` and `body`.
    pub fn route(&self, path: &str, status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> &Self {
        self.route_after(path, Duration::ZERO, status, content_type, body)
    }

    /// Like [`route`](Self::route), answering only after `delay`.
    pub fn route_after(&self, path: &str, delay: Duration, status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> &Self {
        let route = Route { path: path.into(), status, content_type: content_type.into(), body: body.into(), delay, location: None };
        let mut routes = self.routes.lock().unwrap();
        routes.retain(|r| r.path != route.path);
        routes.push(route);
        self
    }

    /// Answer `path` with a redirect (`status` 301/302/303/307/308) to `location`.
    pub fn redirect(&self, path: &str, status: u16, location: &str) -> &Self {
        self.route(path, status, "text/plain", "moved");
        if let Some(route) = self.routes.lock().unwrap().iter_mut().find(|r| r.path == path) {
            route.location = Some(location.into());
        }
        self
    }

    /// Every request so far, oldest first.
    pub fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    /// Requests whose path is `path`.
    pub fn hits_on(&self, path: &str) -> Vec<Hit> {
        self.hits().into_iter().filter(|h| h.path() == path).collect()
    }

    async fn serve(self, mut sock: tokio::net::TcpStream) {
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        let head_end = loop {
            if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            match sock.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
            }
        };
        let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
        let mut lines = head.lines();
        let mut first = lines.next().unwrap_or_default().split_whitespace();
        let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let want: usize = headers.iter().find(|(n, _)| n == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
        let mut body = raw[head_end + 4..].to_vec();
        while body.len() < want {
            match sock.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => body.extend_from_slice(&buf[..n]),
            }
        }
        let hit = Hit { method, target, headers, body: String::from_utf8_lossy(&body).into_owned() };
        let route = self.routes.lock().unwrap().iter().find(|r| r.path == hit.path()).cloned();
        self.hits.lock().unwrap().push(hit);

        let route = route.unwrap_or(Route {
            path: String::new(),
            status: 404,
            content_type: "text/plain".into(),
            body: b"not found".to_vec(),
            delay: Duration::ZERO,
            location: None,
        });
        tokio::time::sleep(route.delay).await;
        let location = route.location.as_ref().map(|l| format!("Location: {l}\r\n")).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\n{location}Connection: close\r\n\r\n",
            route.status,
            route.content_type,
            route.body.len()
        );
        let _ = sock.write_all(head.as_bytes()).await;
        let _ = sock.write_all(&route.body).await;
        let _ = sock.shutdown().await;
    }
}
