//! The adapters: each turns one place (an API, a feed, a page) into [`Item`]s. They never touch the
//! store or the disk (the collector does), only the network through [`Http`], and the addresses
//! they request come from code, the user's own sources, or (read-only, http(s), public hosts) the
//! pages and feeds they fetch; text found on a page never decides what is fetched next.

pub mod hackernews;
pub mod page;
pub mod person;
pub mod rss;
pub mod yc;
pub mod youtube;

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::config::{CollectLimits, Endpoints, TrendConfig};
use super::http::{checked_url, Http};
use super::period::Period;

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
    /// A person the user follows (`name`): Hacker News stories that name them, with the linked
    /// interview, talk or article read, and, when `url` is set, a feed or page of their own.
    Person,
}

impl SourceKind {
    pub const ALL: [SourceKind; 5] = [SourceKind::HackerNews, SourceKind::Yc, SourceKind::Rss, SourceKind::Page, SourceKind::Person];

    /// As stored in `sources.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::HackerNews => "hackernews",
            SourceKind::Yc => "yc",
            SourceKind::Rss => "rss",
            SourceKind::Page => "page",
            SourceKind::Person => "person",
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
            SourceKind::Person => "Person to follow",
        }
    }

    /// Whether the user gives an address (feeds and pages) or the adapter knows where to look.
    pub fn needs_url(self) -> bool {
        matches!(self, SourceKind::Rss | SourceKind::Page)
    }

    /// Whether the user names who to follow (a person; an address of their own is optional).
    pub fn needs_name(self) -> bool {
        matches!(self, SourceKind::Person)
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
    /// Reads what plain HTTP cannot (a video's transcript) in the user's Chrome. `None`: videos
    /// are left for a later run.
    pub videos: Option<youtube::Videos>,
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

/// New items only: not in the store, not seen earlier in `items`.
pub(crate) fn new_items(ctx: &Ctx, items: Vec<Item>) -> Vec<Item> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|i| !ctx.is_known(&i.url) && seen.insert(i.url.clone())).collect()
}

/// The new items (see [`new_items`]) up to `cap`, and how many new items past `cap` were cut. The
/// collector records the count (`SourceOutcome::cut`), so a cap never hides data silently.
pub(crate) fn take_new(ctx: &Ctx, items: Vec<Item>, cap: usize) -> (Vec<Item>, usize) {
    let mut new = new_items(ctx, items);
    let cut = new.len().saturating_sub(cap);
    new.truncate(cap);
    (new, cut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trends::testutil::{ctx, Fixtures};

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

    fn item(url: &str) -> Item {
        Item { url: url.into(), title: url.into(), published_at: None, text: format!("# {url}") }
    }

    #[tokio::test]
    async fn take_new_skips_known_and_repeated_items_and_counts_what_the_cap_cuts() {
        let fx = Fixtures::start().await;
        let ctx = ctx(&fx, &["https://a.test/known"]);
        let items = || ["https://a.test/1", "https://a.test/known", "https://a.test/2", "https://a.test/1", "https://a.test/3", "https://a.test/4"].map(item).to_vec();

        let (kept, cut) = take_new(&ctx, items(), 2);
        assert_eq!(kept.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(), ["https://a.test/1", "https://a.test/2"]);
        assert_eq!(cut, 2, "the known and the repeated item are not cut, they were never new");

        let (kept, cut) = take_new(&ctx, items(), 4);
        assert_eq!((kept.len(), cut), (4, 0), "everything new fits");
        let (kept, cut) = take_new(&ctx, items(), 0);
        assert_eq!((kept.len(), cut), (0, 4));
        assert_eq!(new_items(&ctx, items()).len(), 4);
    }
}
