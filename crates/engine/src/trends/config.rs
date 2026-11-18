//! Where the adapters send requests and how hard they may push: injectable, so tests point them
//! at a local fixture server and a short timeout.

use std::time::Duration;

use super::period::Period;
use crate::net::http::HttpLimits;

/// Base URLs of the APIs the adapters call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// HN Algolia API root (`<root>/search`, `<root>/search_by_date`, `<root>/items/<id>`).
    pub hackernews: String,
    /// Root of the YC company directory mirror: `<root>/meta.json`, `<root>/batches/<slug>.json`.
    /// YC publishes no JSON API of its own; the default is the community `yc-oss` mirror
    /// (static files, refreshed daily from the directory).
    pub yc: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            hackernews: "https://hn.algolia.com/api/v1".into(),
            yc: "https://yc-oss.github.io/api".into(),
        }
    }
}

/// Politeness limits and item caps of one collection run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectLimits {
    /// Minimum gap between two requests to the same host.
    pub host_delay: Duration,
    /// Timeout of one request.
    pub request_timeout: Duration,
    /// Largest response body read (PDFs are the big ones).
    pub max_bytes: usize,
    /// Longest text kept per item (characters); the rest is cut.
    pub max_text_chars: usize,
    /// New items a feed or the YC directory adds per run: what is past it is cut, and the cut is
    /// counted (`SourceOutcome::cut`). Hacker News has no item cap: its stories are bounded by
    /// [`hn_story_pages`](Self::hn_story_pages) pages of [`hn_period_stories`](Self::hn_period_stories),
    /// and every posting of every hiring thread read ([`hiring_threads_for`](Self::hiring_threads_for))
    /// is kept.
    pub max_items: usize,
    /// Front-page stories read per run.
    pub hn_front_page: usize,
    /// Stories of the period asked for in one request (one page of the Algolia search, which
    /// allows up to 1000; `points >= hn_min_points`).
    pub hn_period_stories: usize,
    /// Pages of the period's stories read per run at most. Past it the rest is cut and counted.
    pub hn_story_pages: usize,
    pub hn_min_points: u32,
    /// "Who is hiring" threads read per run for a week, a month or a quarter (see
    /// [`hiring_threads_for`](Self::hiring_threads_for) for a year): the newest that started before
    /// the period's end, one a month. Six fill the digest's six-month skill window on the first run;
    /// later runs skip the postings already stored, so they read only the new threads' postings.
    pub hiring_threads: usize,
    /// Newest YC batches read.
    pub yc_batches: usize,
    /// Feed entries per run whose page is fetched because the feed carries only a teaser.
    pub article_fetches: usize,
    /// One source (a feed, an API) may take this long; past it the source is left out of the run.
    /// Hacker News is the slowest: up to `hn_story_pages` + 2 + `hiring_threads_for` requests one
    /// `host_delay` apart, and a whole hiring thread is a few megabytes. It stops starting pages and
    /// threads once two thirds of this are spent and keeps what it has read (the rest is counted as
    /// cut).
    pub source_timeout: Duration,
    /// One collector (all its sources together) may take this long.
    pub collector_budget: Duration,
    /// Fetch addresses taken from third-party data (feed entries) even when they
    /// point at this machine or a private network. Off, except in tests.
    pub allow_private_hosts: bool,
}

impl Default for CollectLimits {
    fn default() -> Self {
        Self {
            host_delay: Duration::from_secs(1),
            request_timeout: Duration::from_secs(30),
            max_bytes: 20 * 1024 * 1024,
            max_text_chars: 100_000,
            max_items: 100,
            hn_front_page: 30,
            hn_period_stories: 100,
            hn_story_pages: 10,
            hn_min_points: 100,
            hiring_threads: 6,
            yc_batches: 2,
            article_fetches: 20,
            source_timeout: Duration::from_secs(180),
            collector_budget: Duration::from_secs(240),
            allow_private_hosts: false,
        }
    }
}

impl CollectLimits {
    /// "Who is hiring" threads to read for `period`: [`hiring_threads`](Self::hiring_threads) for a
    /// week, a month or a quarter; four times as many for a year (twelve months of the year, twelve
    /// before it as a baseline for the skill trends).
    pub fn hiring_threads_for(&self, period: &Period) -> usize {
        match period {
            Period::Year { .. } => self.hiring_threads.saturating_mul(4),
            Period::Week { .. } | Period::Month { .. } | Period::Quarter { .. } => self.hiring_threads,
        }
    }

    /// What the shared HTTP client takes from these limits.
    pub fn http(&self) -> HttpLimits {
        HttpLimits {
            host_delay: self.host_delay,
            request_timeout: self.request_timeout,
            max_bytes: self.max_bytes,
            allow_private_hosts: self.allow_private_hosts,
        }
    }
}

/// Everything `Core::collect_trends` can be told besides the period.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrendConfig {
    pub endpoints: Endpoints,
    pub limits: CollectLimits,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_year_reads_four_times_the_hiring_threads_of_a_month() {
        let limits = CollectLimits::default();
        let of = |label: &str| limits.hiring_threads_for(&Period::parse(label).unwrap());
        assert_eq!([of("2026-W46"), of("2026-M11"), of("2026-Q4")], [6, 6, 6]);
        assert_eq!(of("2026"), 24, "twelve months and twelve before them");
        let few = CollectLimits { hiring_threads: 2, ..CollectLimits::default() };
        assert_eq!(few.hiring_threads_for(&Period::parse("2026").unwrap()), 8);
    }
}
