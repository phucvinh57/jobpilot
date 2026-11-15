//! The adapters: each turns one place (an API, a feed, a page) into [`Item`]s. They never touch the
//! store or the disk (the collector does), only the network through [`Http`], and the addresses
//! they request come from code, the user's own sources, or (read-only, http(s), public hosts) the
//! pages and search results they fetch; text found on a page never decides what is fetched next.

pub mod hackernews;
pub mod page;
pub mod report_search;
pub mod rss;
pub mod yc;

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::config::{CollectLimits, Endpoints, TrendConfig};
use super::http::{checked_url, Http};
use super::period::Period;

pub use report_search::SearchProvider;

/// The adapter a `sources` row (purpose `trends`) names in `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// Top stories and "Who is hiring" threads (HN Algolia API). No URL needed.
    HackerNews,
    /// The YC company directory (newest batches). No URL needed.
    Yc,
    /// An RSS or Atom feed at `url`.
    Rss,
    /// A single page or PDF at `url`, read like a job description.
    Page,
}

impl SourceKind {
    pub const ALL: [SourceKind; 4] = [SourceKind::HackerNews, SourceKind::Yc, SourceKind::Rss, SourceKind::Page];

    /// As stored in `sources.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::HackerNews => "hackernews",
            SourceKind::Yc => "yc",
            SourceKind::Rss => "rss",
            SourceKind::Page => "page",
        }
    }

    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == kind)
    }

    /// Short label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::HackerNews => "Hacker News",
            SourceKind::Yc => "YC companies",
            SourceKind::Rss => "RSS / Atom feed",
            SourceKind::Page => "Web page",
        }
    }

    /// Whether the user gives an address (feeds and pages) or the adapter knows where to look.
    pub fn needs_url(self) -> bool {
        matches!(self, SourceKind::Rss | SourceKind::Page)
    }
}

/// One collected thing, before it is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What the digest cites (also the dedup key). Normalised by [`normalize_url`].
    pub url: String,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    /// The text kept under `<data_dir>/trends/`; starts with `# <title>`.
    pub text: String,
}

/// What an adapter needs besides its own source row.
#[derive(Clone)]
pub struct Ctx {
    pub http: Http,
    pub config: Arc<TrendConfig>,
    /// URLs already in `trend_items`: not fetched or collected again.
    pub known: Arc<HashSet<String>>,
    pub period: Period,
    /// Hacker News "Who is hiring" threads (ids) read in full by an earlier run, their postings
    /// stored: not downloaded again, except the newest thread (see [`hackernews::collect`]).
    /// Empty for every other adapter.
    pub settled_threads: Arc<HashSet<String>>,
}

impl Ctx {
    pub fn limits(&self) -> &CollectLimits {
        &self.config.limits
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.config.endpoints
    }

    pub fn is_known(&self, url: &str) -> bool {
        self.known.contains(url)
    }
}

/// The form of `raw` items are stored and compared under: http(s) with a host, no fragment.
pub fn normalize_url(raw: &str) -> Result<String> {
    let mut url = checked_url(raw)?;
    url.set_fragment(None);
    Ok(url.to_string())
}

/// `base` + `/` + `path`, whatever the slashes on either side.
pub(crate) fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// New items only: not in the store, not seen earlier in `items`, at most `cap`.
pub(crate) fn take_new(ctx: &Ctx, items: Vec<Item>, cap: usize) -> Vec<Item> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|i| !ctx.is_known(&i.url) && seen.insert(i.url.clone())).take(cap).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_urls_normalise() {
        for kind in SourceKind::ALL {
            assert_eq!(SourceKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SourceKind::parse("greenhouse"), None);
        assert_eq!(normalize_url(" https://a.test#top ").unwrap(), "https://a.test/");
        assert_eq!(normalize_url("https://a.test/x?y=1#frag").unwrap(), "https://a.test/x?y=1");
        assert!(normalize_url("mailto:a@b.c").is_err());
        assert_eq!(join("https://a.test/api/", "/meta.json"), "https://a.test/api/meta.json");
    }
}
