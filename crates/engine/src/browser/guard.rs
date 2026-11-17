//! Which hosts a guarded browser tab (job discovery) may reach.
//!
//! A page from the internet must not be able to make the browser reach this machine or the local
//! network (its document, a redirect, an image, a script, a frame, a worker, a `fetch()` that posts
//! the answer somewhere else). The browser bridge pauses every request of a guarded tab and asks
//! [`allows`] (see `extension/PROTOCOL.md`, "The guard"); the extension also blocks literal private
//! addresses itself.

use std::time::Duration;

use crate::net::http::{ip_is_public, is_public_host};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
