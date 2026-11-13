//! What keeps a discovery run polite and bounded: the per-host delay, the pages per run, the
//! number of sources searched at once, and the "three challenges in a row" rule.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Three challenges in a row on one source disable it for the rest of the day.
pub const MAX_CHALLENGES_IN_A_ROW: u32 = 3;

/// How far a run may scan beyond its productive pages: a page whose postings are all known costs
/// nothing but a request, so `pages_per_run` counts only pages that brought something new, and
/// this many times as many pages may be looked at in total.
pub(crate) const SCAN_FACTOR: u32 = 10;

/// Limits of one discovery run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscoverLimits {
    /// Minimum time between two requests to the same host. Default 1 second.
    pub host_delay: Duration,
    /// Pages per source that bring new postings (see [`SCAN_FACTOR`]). Default 3.
    pub pages_per_run: u32,
    /// Postings per page, so the number of new postings per source and run is at most
    /// `pages_per_run * page_size` (one LLM call each). Default 25.
    pub page_size: usize,
    /// Sources searched at the same time. Default 3.
    pub sources_in_parallel: usize,
}

impl Default for DiscoverLimits {
    fn default() -> Self {
        Self { host_delay: Duration::from_secs(1), pages_per_run: 3, page_size: 25, sources_in_parallel: 3 }
    }
}

/// Spaces out requests to the same host: [`wait`](HostGate::wait) returns at once for a host that
/// was not asked recently, and otherwise after `delay` since the previous asker's turn. Callers
/// queue up in order.
pub struct HostGate {
    delay: Duration,
    next: Mutex<HashMap<String, Instant>>,
}

impl HostGate {
    pub fn new(delay: Duration) -> Self {
        Self { delay, next: Mutex::new(HashMap::new()) }
    }

    /// Wait for this host's turn.
    pub async fn wait(&self, host: &str) {
        if self.delay.is_zero() {
            return;
        }
        let host = host.to_ascii_lowercase();
        let now = Instant::now();
        let turn = {
            let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
            let turn = next.get(&host).copied().filter(|t| *t > now).unwrap_or(now);
            next.insert(host, turn + self.delay);
            turn
        };
        let wait = turn.saturating_duration_since(now);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requests_to_one_host_are_spaced_and_other_hosts_are_not_held() {
        let gate = HostGate::new(Duration::from_millis(60));
        let start = Instant::now();
        gate.wait("a.test").await;
        assert!(start.elapsed() < Duration::from_millis(40), "the first request goes at once");
        gate.wait("b.test").await;
        assert!(start.elapsed() < Duration::from_millis(40), "another host is independent");
        gate.wait("a.test").await;
        assert!(start.elapsed() >= Duration::from_millis(55), "the second request to a.test waits");
    }

    #[tokio::test]
    async fn concurrent_askers_take_turns() {
        let gate = std::sync::Arc::new(HostGate::new(Duration::from_millis(40)));
        let start = Instant::now();
        let tasks: Vec<_> = (0..3)
            .map(|_| {
                let gate = gate.clone();
                tokio::spawn(async move { gate.wait("a.test").await })
            })
            .collect();
        for t in tasks {
            let _ = t.await;
        }
        assert!(start.elapsed() >= Duration::from_millis(75), "three askers need two delays");
    }

    #[tokio::test]
    async fn a_zero_delay_never_waits() {
        let gate = HostGate::new(Duration::ZERO);
        let start = Instant::now();
        for _ in 0..50 {
            gate.wait("a.test").await;
        }
        assert!(start.elapsed() < Duration::from_millis(100));
    }
}
