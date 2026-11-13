//! Where the adapters send requests and how hard they may push: injectable, so tests point them
//! at a local fixture server and a short timeout.

use std::time::Duration;

/// Base URLs of the APIs the adapters call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// HN Algolia API root (`<root>/search`, `<root>/search_by_date`, `<root>/items/<id>`).
    pub hackernews: String,
    /// Root of the YC company directory mirror: `<root>/meta.json`, `<root>/batches/<slug>.json`.
    /// YC publishes no JSON API of its own; the default is the community `yc-oss` mirror
    /// (static files, refreshed daily from the directory).
    pub yc: String,
    /// Brave Search API host (`<host>/res/v1/web/search`).
    pub brave: String,
    /// Tavily API host (`<host>/search`).
    pub tavily: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            hackernews: "https://hn.algolia.com/api/v1".into(),
            yc: "https://yc-oss.github.io/api".into(),
            brave: "https://api.search.brave.com".into(),
            tavily: "https://api.tavily.com".into(),
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
    /// Largest response body read (PDF reports are the big ones).
    pub max_bytes: usize,
    /// Longest text kept per item (characters); the rest is cut.
    pub max_text_chars: usize,
    /// New items one source adds per run.
    pub max_items: usize,
    /// Front-page stories read per run.
    pub hn_front_page: usize,
    /// Stories of the period read per run (`points >= hn_min_points`).
    pub hn_period_stories: usize,
    pub hn_min_points: u32,
    /// Postings kept from the newest "Who is hiring" thread.
    pub hiring_posts: usize,
    /// Newest YC batches read.
    pub yc_batches: usize,
    /// Feed entries per run whose page is fetched because the feed carries only a teaser.
    pub article_fetches: usize,
    /// Report-search queries per run.
    pub max_queries: usize,
    /// Results read per query.
    pub results_per_query: usize,
    /// One source (a feed, an API) may take this long; past it the source is left out of the run.
    pub source_timeout: Duration,
    /// One collector (all its sources together) may take this long.
    pub collector_budget: Duration,
    /// Fetch addresses taken from third-party data (search results, feed entries) even when they
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
            hn_period_stories: 30,
            hn_min_points: 100,
            hiring_posts: 150,
            yc_batches: 2,
            article_fetches: 20,
            max_queries: 6,
            results_per_query: 5,
            source_timeout: Duration::from_secs(120),
            collector_budget: Duration::from_secs(240),
            allow_private_hosts: false,
        }
    }
}

/// Everything `Core::collect_trends` can be told besides the period.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrendConfig {
    pub endpoints: Endpoints,
    pub limits: CollectLimits,
}
