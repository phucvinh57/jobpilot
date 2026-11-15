//! Hacker News through the Algolia API: the front page, the period's most upvoted stories, and the
//! newest "Ask HN: Who is hiring?" threads before the period's end (one item per posting; several
//! monthly threads, so the digest's skill window is full from the first run).
//!
//! A story's item is the story itself (its link, or the HN thread for text posts): the title and
//! the numbers are the text; the linked article is not fetched.

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use super::{join, normalize_url, take_new, Ctx, Item};
use crate::engine::trends::text::{first_line, html_to_text};

/// Permalink of an HN item (story or comment).
const ITEM_URL: &str = "https://news.ycombinator.com/item?id=";
/// A hiring comment shorter than this is a reply or a stray remark, not a posting.
const MIN_POSTING_CHARS: usize = 40;

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    hits: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    #[serde(rename = "objectID")]
    object_id: String,
    title: Option<String>,
    url: Option<String>,
    story_text: Option<String>,
    points: Option<i64>,
    num_comments: Option<i64>,
    created_at_i: Option<i64>,
}

#[derive(Deserialize)]
struct Thread {
    #[serde(default)]
    children: Vec<Comment>,
}

#[derive(Deserialize)]
struct Comment {
    id: u64,
    text: Option<String>,
    created_at_i: Option<i64>,
}

/// Stories and hiring postings. A part that fails is left out; the adapter fails only when every
/// part did.
pub async fn collect(ctx: &Ctx) -> Result<Vec<Item>> {
    let started = std::time::Instant::now();
    let mut items = Vec::new();
    let mut failures = Vec::new();
    for part in [front_page(ctx).await, period_stories(ctx).await] {
        match part {
            Ok(found) => items.extend(found),
            Err(e) => failures.push(e),
        }
    }
    let mut items = take_new(ctx, items, ctx.limits().max_items);
    let hiring = hiring_posts(ctx, started).await;
    match hiring {
        Ok(found) => items.extend(found),
        Err(e) => failures.push(e),
    }
    match failures.into_iter().next() {
        Some(first) if items.is_empty() => Err(first),
        Some(_) => {
            tracing::warn!(kept = items.len(), "a Hacker News request failed; the other parts are kept");
            Ok(items)
        }
        None => Ok(items),
    }
}

async fn front_page(ctx: &Ctx) -> Result<Vec<Item>> {
    let n = ctx.limits().hn_front_page;
    let url = api_url(ctx, "search", &[("tags", "front_page"), ("hitsPerPage", &n.to_string())])?;
    stories(ctx, &url).await
}

async fn period_stories(ctx: &Ctx) -> Result<Vec<Item>> {
    let l = ctx.limits();
    let filters = format!(
        "created_at_i>={},created_at_i<{},points>={}",
        ctx.period.start().timestamp(),
        ctx.period.end().timestamp(),
        l.hn_min_points
    );
    let url = api_url(
        ctx,
        "search_by_date",
        &[("tags", "story"), ("numericFilters", &filters), ("hitsPerPage", &l.hn_period_stories.to_string())],
    )?;
    stories(ctx, &url).await
}

async fn stories(ctx: &Ctx, url: &str) -> Result<Vec<Item>> {
    let search: Search = ctx.http.get(url, &[]).await?.json("Hacker News")?;
    Ok(search.hits.into_iter().filter_map(story_item).collect())
}

fn story_item(hit: Hit) -> Option<Item> {
    let title = hit.title.as_deref().map(str::trim).filter(|t| !t.is_empty())?.to_string();
    let discussion = format!("{ITEM_URL}{}", hit.object_id);
    let url = hit.url.as_deref().and_then(|u| normalize_url(u).ok()).or_else(|| normalize_url(&discussion).ok())?;
    let mut text = format!(
        "# {title}\n\n{} points and {} comments on Hacker News. Discussion: {discussion}",
        hit.points.unwrap_or(0),
        hit.num_comments.unwrap_or(0)
    );
    if let Some(body) = hit.story_text.as_deref().map(html_to_text).filter(|b| !b.is_empty()) {
        text.push_str("\n\n");
        text.push_str(&body);
    }
    Some(Item { url, title, published_at: timestamp(hit.created_at_i), text })
}

/// The newest "Who is hiring" threads that started before the period ended (at most
/// [`CollectLimits::hiring_threads`](crate::engine::trends::CollectLimits), newest first), one item
/// per posting. Reading several monthly threads fills the skill window on the first run; postings
/// already stored are skipped (`take_new`), so a later run reads only what is new. At most
/// `hiring_posts` new postings per thread. A thread that cannot be read is left out; this part
/// fails only when every thread did.
async fn hiring_posts(ctx: &Ctx, started: std::time::Instant) -> Result<Vec<Item>> {
    let l = ctx.limits();
    // The whole source runs under `source_timeout`, and a timeout loses everything it read: stop
    // starting threads once two thirds of it are spent, and keep what is in hand.
    let budget = l.source_timeout.mul_f64(BACKFILL_SHARE);
    let end = ctx.period.end().timestamp();
    // The bot posts three threads a month ("Who is hiring", "Who wants to be hired", "Freelancer"):
    // ask for enough to see `hiring_threads` hiring ones, only threads that started before the end.
    let hits = l.hiring_threads.saturating_mul(3).saturating_add(3).clamp(12, 100).to_string();
    let before_end = format!("created_at_i<{end}");
    let list = api_url(
        ctx,
        "search_by_date",
        &[("tags", "story,author_whoishiring"), ("numericFilters", &before_end), ("hitsPerPage", &hits)],
    )?;
    let threads: Search = ctx.http.get(&list, &[]).await?.json("Hacker News")?;
    let threads = hiring_threads(threads.hits, end, l.hiring_threads);

    let mut items = Vec::new();
    let (mut read, mut first_error) = (0usize, None);
    for (n, thread) in threads.iter().enumerate() {
        if n > 0 && started.elapsed() >= budget {
            tracing::warn!(left = threads.len() - n, "time is running out: the older hiring threads are left for a later run");
            break;
        }
        match thread_posts(ctx, thread).await {
            Ok(found) => {
                read += 1;
                items.extend(found);
            }
            Err(e) => {
                tracing::warn!(thread = %thread.object_id, "a hiring thread could not be read and is left out");
                first_error.get_or_insert(e);
            }
        }
    }
    tracing::info!(threads = threads.len(), read, postings = items.len(), "hiring threads read");
    match first_error {
        Some(e) if read == 0 => Err(e),
        _ => Ok(items),
    }
}

/// The part of the source's timeout after which no further hiring thread is started.
const BACKFILL_SHARE: f64 = 2.0 / 3.0;

/// The "Who is hiring" threads among `hits` that started before `end`, newest first, at most `max`.
fn hiring_threads(hits: Vec<Hit>, end: i64, max: usize) -> Vec<Hit> {
    let mut threads: Vec<Hit> = hits
        .into_iter()
        .filter(|h| h.title.as_deref().is_some_and(|t| t.to_lowercase().contains("who is hiring")))
        .filter(|h| h.created_at_i.is_some_and(|t| t < end))
        .collect();
    threads.sort_by_key(|h| std::cmp::Reverse(h.created_at_i));
    let mut seen = std::collections::HashSet::new();
    threads.retain(|h| seen.insert(h.object_id.clone()));
    threads.truncate(max);
    threads
}

/// The new postings of one thread (at most `hiring_posts`). Each keeps the thread's title in its
/// text header (`# Hacker News, <title>`): the digest recognises hiring postings by it.
async fn thread_posts(ctx: &Ctx, thread: &Hit) -> Result<Vec<Item>> {
    let title = thread.title.clone().unwrap_or_default();
    let full: Thread = ctx
        .http
        .get(&join(&ctx.endpoints().hackernews, &format!("items/{}", thread.object_id)), &[])
        .await?
        .json("Hacker News")
        .context("reading a hiring thread")?;

    let postings = full.children.into_iter().filter_map(|c| {
        let text = html_to_text(c.text.as_deref()?);
        if text.chars().count() < MIN_POSTING_CHARS {
            return None;
        }
        let headline = first_line(&text, 120)?;
        Some(Item {
            url: format!("{ITEM_URL}{}", c.id),
            title: headline,
            published_at: timestamp(c.created_at_i),
            text: format!("# Hacker News, {title}\n\n{text}"),
        })
    });
    Ok(take_new(ctx, postings.collect(), ctx.limits().hiring_posts))
}

fn api_url(ctx: &Ctx, path: &str, query: &[(&str, &str)]) -> Result<String> {
    let mut url = url::Url::parse(&join(&ctx.endpoints().hackernews, path)).context("the Hacker News API address is not a URL")?;
    url.query_pairs_mut().extend_pairs(query);
    Ok(url.to_string())
}

fn timestamp(secs: Option<i64>) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(secs?, 0).single()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use std::time::{Duration, Instant};

    use super::*;
    use crate::engine::trends::testutil::{ctx, Fixtures};

    const FRONT: &str = r#"{"hits": [
        {"objectID": "1", "title": "Rust 2.0 released", "url": "https://blog.example.com/rust-2#intro", "points": 450, "num_comments": 210, "created_at_i": 1794000000, "story_text": null},
        {"objectID": "2", "title": "Ask HN: Is anyone hiring SREs?", "url": null, "story_text": "<p>Looking for <i>SRE</i> roles &amp; teams.", "points": 12, "num_comments": 3, "created_at_i": 1794000100},
        {"objectID": "3", "title": "  ", "url": "https://x.test/untitled", "points": 1, "created_at_i": 1794000200},
        {"objectID": "4", "title": "A bad link", "url": "javascript:alert(1)", "points": 5, "created_at_i": 1794000300}
    ]}"#;

    const PERIOD: &str = r#"{"hits": [
        {"objectID": "1", "title": "Rust 2.0 released", "url": "https://blog.example.com/rust-2", "points": 450, "num_comments": 210, "created_at_i": 1794000000},
        {"objectID": "5", "title": "Postgres 19 is out", "url": "https://pg.example.org/19", "points": 300, "num_comments": 90, "created_at_i": 1794100000}
    ]}"#;

    const THREADS: &str = r#"{"hits": [
        {"objectID": "900", "title": "Ask HN: Who wants to be hired? (November 2026)", "created_at_i": 1793635297},
        {"objectID": "901", "title": "Ask HN: Who is hiring? (December 2026)", "created_at_i": 1796300000},
        {"objectID": "904", "title": "Ask HN: Who is hiring? (September 2026)", "created_at_i": 1788274800},
        {"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297},
        {"objectID": "903", "title": "Ask HN: Who is hiring? (October 2026)", "created_at_i": 1790866927},
        {"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297}
    ]}"#;

    const THREAD: &str = r#"{"id": 902, "title": "Ask HN: Who is hiring? (November 2026)", "children": [
        {"id": 7001, "created_at_i": 1793635303, "text": "Acme | Senior Rust Engineer | REMOTE | &#x24;150k<p>We build robots in Rust &amp; Go. Apply at <a href=\"https:&#x2F;&#x2F;acme.test&#x2F;jobs\">https:&#x2F;&#x2F;acme.test&#x2F;jobs</a>", "children": [{"id": 7002, "text": "reply", "children": []}]},
        {"id": 7003, "created_at_i": 1793635400, "text": "short reply", "children": []},
        {"id": 7004, "created_at_i": 1793635500, "text": null, "children": []},
        {"id": 7005, "created_at_i": 1793635600, "text": "Globex | Data Engineer | Berlin, ONSITE | Python, Spark, Kafka, dbt, Airflow, Snowflake", "children": []}
    ]}"#;

    const THREAD_OCT: &str = r#"{"id": 903, "children": [
        {"id": 8001, "created_at_i": 1790867000, "text": "Initech | Platform Engineer | Singapore, ONSITE | Kubernetes, Terraform, Go", "children": []},
        {"id": 8002, "created_at_i": 1790867100, "text": "Hooli | ML Engineer | REMOTE (US) | Python, PyTorch, Ray, Kubernetes", "children": []}
    ]}"#;

    const THREAD_SEP: &str = r#"{"id": 904, "children": [
        {"id": 9001, "created_at_i": 1788275000, "text": "Umbrella | Backend Engineer | Ho Chi Minh City | Java, Spring, Postgres, Kafka", "children": []}
    ]}"#;

    /// The thread list and every thread. The list and the period's stories share /search_by_date:
    /// tests of `hiring_posts` alone serve the list there.
    fn hiring_fixtures(fx: &Fixtures) {
        fx.route("/hn/search_by_date", 200, "application/json", THREADS);
        fx.route("/hn/items/902", 200, "application/json", THREAD);
        fx.route("/hn/items/903", 200, "application/json", THREAD_OCT);
        fx.route("/hn/items/904", 200, "application/json", THREAD_SEP);
    }

    /// An adapter context with `known` already collected and the hiring limits changed by `limits`.
    fn ctx_with(fx: &Fixtures, known: &[&str], limits: impl FnOnce(&mut crate::engine::trends::CollectLimits)) -> Ctx {
        let mut c = ctx(fx, known);
        let mut config = (*c.config).clone();
        limits(&mut config.limits);
        c.config = Arc::new(config);
        c
    }

    fn urls(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.url.as_str()).collect()
    }

    fn fixtures(fx: &Fixtures) {
        fx.route("/hn/search", 200, "application/json", FRONT);
        fx.route("/hn/search_by_date", 200, "application/json", PERIOD);
    }

    #[tokio::test]
    async fn stories_become_items_with_their_link_or_thread() {
        let fx = Fixtures::start().await;
        fixtures(&fx);
        let ctx = ctx(&fx, &[]);

        let items = front_page(&ctx).await.unwrap();
        assert_eq!(items.len(), 3, "the untitled story is dropped");
        assert_eq!(items[0].url, "https://blog.example.com/rust-2", "the story's own link, fragment removed");
        assert_eq!(items[0].title, "Rust 2.0 released");
        assert!(items[0].text.starts_with("# Rust 2.0 released\n\n450 points and 210 comments"), "{}", items[0].text);
        assert!(items[0].text.contains("https://news.ycombinator.com/item?id=1"));
        assert_eq!(items[0].published_at.unwrap().timestamp(), 1794000000);
        assert_eq!(items[1].url, "https://news.ycombinator.com/item?id=2", "text posts cite the thread");
        assert!(items[1].text.contains("Looking for SRE roles & teams."), "{}", items[1].text);
        assert_eq!(items[2].url, "https://news.ycombinator.com/item?id=4", "a link that is not http(s) is never cited: the thread is");

        period_stories(&ctx).await.unwrap();
        let hit = &fx.hits_on("/hn/search")[0];
        assert!(hit.target.contains("tags=front_page") && hit.target.contains("hitsPerPage=30"), "{}", hit.target);
        let by_date = &fx.hits_on("/hn/search_by_date")[0];
        // 2026-W46 = 2026-11-09 .. 2026-11-16 (UTC)
        assert!(by_date.target.contains("created_at_i%3E%3D1794182400"), "{}", by_date.target);
        assert!(by_date.target.contains("created_at_i%3C1794787200"), "{}", by_date.target);
        assert!(by_date.target.contains("points%3E%3D100"), "{}", by_date.target);
    }

    #[tokio::test]
    async fn hiring_reads_the_newest_threads_before_the_period_end_one_item_per_posting() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let ctx = ctx(&fx, &[]);

        let posts = hiring_posts(&ctx, Instant::now()).await.unwrap();
        // Newest thread first; December's thread starts after 2026-W46, "Who wants to be hired" is
        // another thread, and a thread listed twice is read once.
        assert_eq!(
            urls(&posts),
            [
                "https://news.ycombinator.com/item?id=7001",
                "https://news.ycombinator.com/item?id=7005",
                "https://news.ycombinator.com/item?id=8001",
                "https://news.ycombinator.com/item?id=8002",
                "https://news.ycombinator.com/item?id=9001",
            ],
            "replies, empty and deleted comments are not postings"
        );
        for id in ["902", "903", "904"] {
            assert_eq!(fx.hits_on(&format!("/hn/items/{id}")).len(), 1, "thread {id}");
        }
        assert!(fx.hits_on("/hn/items/901").is_empty() && fx.hits_on("/hn/items/900").is_empty());
        let list = &fx.hits_on("/hn/search_by_date")[0];
        assert!(list.target.contains("tags=story%2Cauthor_whoishiring"), "{}", list.target);
        assert!(list.target.contains("numericFilters=created_at_i%3C1794787200"), "only threads before the end: {}", list.target);
        assert!(list.target.contains("hitsPerPage=21"), "three threads a month, six months, and slack: {}", list.target);

        assert_eq!(posts[0].title, "Acme | Senior Rust Engineer | REMOTE | $150k");
        assert!(posts[0].text.starts_with("# Hacker News, Ask HN: Who is hiring? (November 2026)\n\nAcme |"), "{}", posts[0].text);
        assert!(posts[0].text.contains("We build robots in Rust & Go. Apply at https://acme.test/jobs"), "{}", posts[0].text);
        assert_eq!(posts[0].published_at.unwrap().timestamp(), 1793635303);
        assert!(posts[2].text.starts_with("# Hacker News, Ask HN: Who is hiring? (October 2026)\n\nInitech |"), "{}", posts[2].text);
        assert!(posts[4].text.starts_with("# Hacker News, Ask HN: Who is hiring? (September 2026)\n\n"), "{}", posts[4].text);
    }

    #[tokio::test]
    async fn hiring_threads_and_postings_per_thread_are_capped() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);

        let two = ctx_with(&fx, &[], |l| l.hiring_threads = 2);
        let posts = hiring_posts(&two, Instant::now()).await.unwrap();
        assert_eq!(posts.len(), 4);
        assert!(fx.hits_on("/hn/items/904").is_empty(), "the third newest thread is not read");

        // The cap is per thread, and it counts only new postings.
        let one = ctx_with(&fx, &["https://news.ycombinator.com/item?id=7001"], |l| l.hiring_posts = 1);
        let posts = hiring_posts(&one, Instant::now()).await.unwrap();
        assert_eq!(
            urls(&posts),
            ["https://news.ycombinator.com/item?id=7005", "https://news.ycombinator.com/item?id=8001", "https://news.ycombinator.com/item?id=9001"]
        );
    }

    #[tokio::test]
    async fn no_further_thread_starts_once_the_time_is_spent() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let spent = ctx_with(&fx, &[], |l| l.source_timeout = Duration::ZERO);
        let posts = hiring_posts(&spent, Instant::now()).await.unwrap();
        assert!(!posts.is_empty(), "the newest thread is always read");
        assert!(fx.hits_on("/hn/items/903").is_empty(), "the older threads are left for a later run");
    }

    #[tokio::test]
    async fn a_second_run_reads_nothing_new() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let first = hiring_posts(&ctx(&fx, &[]), Instant::now()).await.unwrap();
        assert_eq!(first.len(), 5);

        let known: Vec<&str> = urls(&first);
        let again = hiring_posts(&ctx(&fx, &known), Instant::now()).await.unwrap();
        assert!(again.is_empty(), "every posting is already stored: {:?}", urls(&again));

        // A new posting in the newest thread is all a later run adds.
        fx.route("/hn/items/902", 200, "application/json", THREAD.replace(
            r#"{"id": 7003, "created_at_i": 1793635400, "text": "short reply""#,
            r#"{"id": 7006, "created_at_i": 1793700000, "text": "Vandelay | Rust Engineer | Singapore | Rust, Tokio, Postgres""#,
        ));
        let later = hiring_posts(&ctx(&fx, &known), Instant::now()).await.unwrap();
        assert_eq!(urls(&later), ["https://news.ycombinator.com/item?id=7006"]);
    }

    #[tokio::test]
    async fn a_thread_that_fails_is_left_out_and_all_failing_is_an_error() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        fx.route("/hn/items/903", 500, "text/plain", "boom");
        let posts = hiring_posts(&ctx(&fx, &[]), Instant::now()).await.unwrap();
        assert_eq!(posts.len(), 3, "November and September are kept: {:?}", urls(&posts));

        fx.route("/hn/items/902", 500, "text/plain", "boom");
        fx.route("/hn/items/904", 503, "text/plain", "down");
        let err = hiring_posts(&ctx(&fx, &[]), Instant::now()).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 500") || err.contains("HTTP 503"), "{err}");

        // No hiring thread before the end: nothing, not an error.
        fx.route("/hn/search_by_date", 200, "application/json", r#"{"hits": []}"#);
        assert!(hiring_posts(&ctx(&fx, &[]), Instant::now()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn collect_merges_dedups_and_survives_one_failing_part() {
        let fx = Fixtures::start().await;
        fixtures(&fx);
        // `search_by_date` serves both the period's stories and the thread list: give it a body that
        // is valid for both shapes (hits with a title and no hiring thread).
        let ctx = ctx(&fx, &["https://pg.example.org/19"]);
        let items = collect(&ctx).await.unwrap();
        let urls: Vec<_> = items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://blog.example.com/rust-2", "https://news.ycombinator.com/item?id=2", "https://news.ycombinator.com/item?id=4"],
            "the story that is in both lists once; the known one dropped"
        );

        // The front page fails, the rest is kept.
        fx.route("/hn/search", 500, "text/plain", "boom");
        let items = collect(&ctx).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://blog.example.com/rust-2");

        // Everything fails: an error, not an empty success.
        fx.route("/hn/search_by_date", 503, "text/plain", "down");
        let err = collect(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 500") || err.contains("HTTP 503"), "{err}");
    }
}
