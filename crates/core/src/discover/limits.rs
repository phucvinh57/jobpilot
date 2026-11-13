//! What keeps a discovery run polite and bounded: the per-host delay (kept by the shared HTTP
//! client), the pages per run, the number of sources searched at once, and the "three challenges
//! in a row" rule.

use std::time::Duration;

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
    /// Fetch addresses on this machine or a private network too. Off, except in tests: posting
    /// links come from third-party data.
    pub allow_private_hosts: bool,
}

impl Default for DiscoverLimits {
    fn default() -> Self {
        Self { host_delay: Duration::from_secs(1), pages_per_run: 3, page_size: 25, sources_in_parallel: 3, allow_private_hosts: false }
    }
}
