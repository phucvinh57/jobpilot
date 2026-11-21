//! Hacker News through the Algolia API: the front page, the period's most upvoted stories, and the
//! newest "Ask HN: Who is hiring?" threads before the period's end (one item per posting; several
//! monthly threads, so the digest's skill window is full from the first run; an older thread read
//! in full is not downloaded again until its comment count changes).
//!
//! Nothing is cut silently. The period's stories are read page by page until the last page, up to
//! `hn_story_pages`; every new posting of a hiring thread is kept. What the page cap or the time
//! budget leaves unread of the stories is counted in [`Hn::cut`], the hiring threads not started in
//! [`Hn::threads_left`].
//!
//! A story's item is the story itself (its link, or the HN thread for text posts): the title and
//! the numbers are the text; the linked article is not fetched.

use std::collections::HashSet;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use super::{join, new_items, normalize_url, Ctx, Item};
use crate::trends::text::{first_line, html_to_text};

/// Permalink of an HN item (story or comment).
const ITEM_URL: &str = "https://news.ycombinator.com/item?id=";
/// A hiring comment shorter than this is a reply or a stray remark, not a posting.
const MIN_POSTING_CHARS: usize = 40;

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    hits: Vec<Hit>,
    /// Pages the whole result has at this page size (0 when the answer does not say).
    #[serde(default, rename = "nbPages")]
    nb_pages: usize,
    /// Hits the whole result has, over all pages.
    #[serde(default, rename = "nbHits")]
    nb_hits: usize,
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

/// What the source read.
#[derive(Debug, Default)]
pub struct Hn {
    /// Stories and hiring postings, new ones only.
    pub items: Vec<Item>,
    /// Hiring threads this run read in full, newest first, as [`settled_key`]s: not the newest
    /// selected thread, and downloaded. Once their postings are stored, the collector saves them,
    /// and a later run passes them back to [`collect`] so they are not downloaded again while their
    /// comment count stays the same.
    pub settled: Vec<String>,
    /// Period stories this run left unread: past the page cap, the time budget or the last page
    /// the API serves (the hits the answer says are left, else 1).
    pub cut: usize,
    /// Older hiring threads the time budget kept from starting. They are not part of `cut`: their
    /// postings are unknown, a later run reads them, and until then their months hold no postings,
    /// which the digest reports.
    pub threads_left: usize,
}

/// How a settled thread is remembered: its id and its comment count when it was read
/// (`"903@412"`, the count the thread list gives, which also counts replies and deleted comments).
/// People still post to a thread after the next month's opens; a new comment changes the count, so
/// the thread is read again.
pub fn settled_key(thread: &str, comments: u64) -> String {
    format!("{thread}@{comments}")
}

/// The thread id of a [`settled_key`]; `None` for anything else.
pub fn settled_thread(key: &str) -> Option<u64> {
    let (thread, comments) = key.split_once('@')?;
    comments.parse::<u64>().ok()?;
    thread.parse().ok()
}

/// The settled key of a listed thread, `None` when the list gives no comment count (such a thread
/// is never settled).
fn key_of(thread: &Hit) -> Option<String> {
    thread.num_comments.and_then(|n| u64::try_from(n).ok()).map(|n| settled_key(&thread.object_id, n))
}

/// Stories and hiring postings. A part that fails is left out; the adapter fails only when every
/// part did. `settled` holds the [`settled_key`]s of hiring threads an earlier run read in full.
/// Every new story is kept: the page cap bounds them, not `max_items`.
pub async fn collect(ctx: &Ctx, settled: &HashSet<String>) -> Result<Hn> {
    let started = Instant::now();
    let mut items = Vec::new();
    let mut failures = Vec::new();
    let (mut cut, mut threads_left) = (0, 0);
    match front_page(ctx).await {
        Ok(found) => items.extend(found),
        Err(e) => failures.push(e),
    }
    match period_stories(ctx, started).await {
        Ok((found, unread)) => {
            items.extend(found);
            cut += unread;
        }
        Err(e) => failures.push(e),
    }
    let mut items = new_items(ctx, items);
    let mut now_settled = Vec::new();
    match hiring_posts(ctx, settled, started).await {
        Ok(found) => {
            items.extend(found.items);
            now_settled = found.settled;
            threads_left = found.threads_left;
        }
        Err(e) => failures.push(e),
    }
    match failures.into_iter().next() {
        Some(first) if items.is_empty() => Err(first),
        Some(_) => {
            tracing::warn!(kept = items.len(), "a Hacker News request failed; the other parts are kept");
            Ok(Hn { items, settled: now_settled, cut, threads_left })
        }
        None => Ok(Hn { items, settled: now_settled, cut, threads_left }),
    }
}

async fn front_page(ctx: &Ctx) -> Result<Vec<Item>> {
    let n = ctx.limits().hn_front_page;
    let url = api_url(ctx, "search", &[("tags", "front_page"), ("hitsPerPage", &n.to_string())])?;
    stories(ctx, &url).await
}

/// The period's stories with `points >= hn_min_points`, newest first, read page by page until the
/// last page (`nbPages`), at most `hn_story_pages` pages. Like [`hiring_posts`] it stops starting
/// pages once [`BACKFILL_SHARE`] of the source's time has passed since `started` (the first page is
/// always read) and keeps what is in hand. Returns the stories and how many hits were left unread:
/// the page cap, the time budget or a failing later page cut them. The count is what the answer
/// says is left (`nbHits`), else 1. A failing first page is an error.
async fn period_stories(ctx: &Ctx, started: Instant) -> Result<(Vec<Item>, usize)> {
    let l = ctx.limits();
    let budget = l.source_timeout.mul_f64(BACKFILL_SHARE);
    let filters = format!(
        "created_at_i>={},created_at_i<{},points>={}",
        ctx.period.start().timestamp(),
        ctx.period.end().timestamp(),
        l.hn_min_points
    );
    let size = l.hn_period_stories.to_string();
    let mut items = Vec::new();
    // Hits read so far, and the total the last answer gave.
    let (mut read, mut total) = (0usize, 0usize);
    for page in 0..l.hn_story_pages.max(1) {
        if page > 0 && started.elapsed() >= budget {
            tracing::warn!(page, read, "time is running out: the rest of the period's stories is left unread");
            break;
        }
        let number = page.to_string();
        let url = api_url(
            ctx,
            "search_by_date",
            &[("tags", "story"), ("numericFilters", &filters), ("hitsPerPage", &size), ("page", &number)],
        )?;
        let answer = match search(ctx, &url).await {
            Ok(answer) => answer,
            Err(e) if page == 0 => return Err(e),
            Err(_) => {
                tracing::warn!(page, read, "a page of the period's stories could not be read; the pages before it are kept");
                break;
            }
        };
        read += answer.hits.len();
        total = answer.nb_hits;
        // An empty page that is not the last: nothing more will come.
        let last = page + 1 >= answer.nb_pages || answer.hits.is_empty();
        items.extend(answer.hits.into_iter().filter_map(story_item));
        if last {
            // The API serves only the first hits of a large result: the last page it offers is not
            // always the end of the result.
            let cut = total.saturating_sub(read);
            if cut > 0 {
                tracing::warn!(read, cut, "the period has more stories than the API serves");
            }
            return Ok((items, cut));
        }
    }
    let cut = total.saturating_sub(read).max(1);
    tracing::warn!(pages = l.hn_story_pages, read, cut, "the period's stories were not all read");
    Ok((items, cut))
}

async fn search(ctx: &Ctx, url: &str) -> Result<Search> {
    ctx.http.get(url, &[]).await?.json("Hacker News")
}

async fn stories(ctx: &Ctx, url: &str) -> Result<Vec<Item>> {
    Ok(search(ctx, url).await?.hits.into_iter().filter_map(story_item).collect())
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

/// A story whose title names someone (for the [`person`](super::person) adapter).
pub(super) struct Story {
    /// The story as [`story_item`] makes it: the title and the numbers are the text.
    pub item: Item,
    /// The item's address is the page the story links (not the Hacker News thread of a text post).
    pub linked: bool,
}

/// The stories from `since` to the period's end whose title names `name` (the whole name, as a
/// phrase, whatever its case), newest first: one page of `person_stories` hits with
/// `points >= person_min_points`. The search is asked for the phrase in titles only, and the title
/// is checked again here, so a search that matches loosely adds nothing.
pub(super) async fn stories_naming(ctx: &Ctx, name: &str, since: DateTime<Utc>) -> Result<Vec<Story>> {
    let l = ctx.limits();
    let name = name.replace('"', " ").split_whitespace().collect::<Vec<_>>().join(" ");
    anyhow::ensure!(!name.is_empty(), "a person to follow needs a name");
    let filters = format!(
        "created_at_i>={},created_at_i<{},points>={}",
        since.timestamp(),
        ctx.period.end().timestamp(),
        l.person_min_points
    );
    let url = api_url(
        ctx,
        "search_by_date",
        &[
            ("query", &format!("\"{name}\"")),
            ("tags", "story"),
            ("restrictSearchableAttributes", "title"),
            ("numericFilters", &filters),
            ("hitsPerPage", &l.person_stories.to_string()),
        ],
    )?;
    let hits = search(ctx, &url).await?.hits;
    Ok(hits
        .into_iter()
        .filter_map(|hit| {
            let linked = hit.url.as_deref().is_some_and(|u| normalize_url(u).is_ok());
            let item = story_item(hit)?;
            crate::discover::source::mentions(&item.title, &name).then_some(Story { item, linked })
        })
        .collect())
}

/// The newest "Who is hiring" threads that started before the period ended (at most
/// [`CollectLimits::hiring_threads_for`](crate::trends::CollectLimits) the period, newest first),
/// one item per posting. Reading several monthly threads fills the skill window on the first run;
/// postings already stored are skipped (`new_items`), so a later run reads only what is new. Every
/// new posting of a thread is kept. A thread that cannot be read is left out; this part fails only
/// when every thread did.
///
/// An older thread whose [`settled_key`] is in `settled` (same id, same comment count) is not
/// downloaded: it still takes its place in the window, its postings are already stored. The newest
/// selected thread is always read, since it still gets new postings. [`Hn::settled`] lists the
/// older threads read in full this run; [`Hn::threads_left`] counts the older threads the time budget kept
/// from starting.
async fn hiring_posts(ctx: &Ctx, settled: &HashSet<String>, started: Instant) -> Result<Hn> {
    let l = ctx.limits();
    // The whole source runs under `source_timeout`, and a timeout loses everything it read: stop
    // starting threads once two thirds of it are spent, and keep what is in hand.
    let budget = l.source_timeout.mul_f64(BACKFILL_SHARE);
    let end = ctx.period.end().timestamp();
    let wanted = l.hiring_threads_for(&ctx.period);
    // The bot posts three threads a month ("Who is hiring", "Who wants to be hired", "Freelancer"):
    // ask for enough to see `wanted` hiring ones, only threads that started before the end.
    let hits = wanted.saturating_mul(3).saturating_add(3).clamp(12, 100).to_string();
    let before_end = format!("created_at_i<{end}");
    let list = api_url(
        ctx,
        "search_by_date",
        &[("tags", "story,author_whoishiring"), ("numericFilters", &before_end), ("hitsPerPage", &hits)],
    )?;
    let threads = search(ctx, &list).await?;
    let threads = hiring_threads(threads.hits, end, wanted);

    let is_settled = |t: &Hit| key_of(t).is_some_and(|k| settled.contains(&k));
    let mut items = Vec::new();
    let mut now_settled = Vec::new();
    let (mut read, mut skipped, mut threads_left, mut first_error) = (0usize, 0usize, 0usize, None);
    for (n, thread) in threads.iter().enumerate() {
        let newest = n == 0;
        if !newest && is_settled(thread) {
            skipped += 1;
            continue;
        }
        if !newest && started.elapsed() >= budget {
            threads_left = threads[n..].iter().filter(|t| !is_settled(t)).count();
            tracing::warn!(left = threads_left, "time is running out: the older hiring threads are left for a later run");
            break;
        }
        match thread_posts(ctx, thread).await {
            Ok(found) => {
                read += 1;
                items.extend(found);
                if !newest {
                    now_settled.extend(key_of(thread));
                }
            }
            Err(e) => {
                tracing::warn!(thread = %thread.object_id, "a hiring thread could not be read and is left out");
                first_error.get_or_insert(e);
            }
        }
    }
    tracing::info!(threads = threads.len(), read, skipped, postings = items.len(), threads_left, "hiring threads read");
    match first_error {
        Some(e) if read == 0 => Err(e),
        _ => Ok(Hn { items, settled: now_settled, cut: 0, threads_left }),
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
    let mut seen = HashSet::new();
    threads.retain(|h| seen.insert(h.object_id.clone()));
    threads.truncate(max);
    threads
}

/// The new postings of one thread, all of them (it arrives in one response). Each keeps the
/// thread's title in its text header (`# Hacker News, <title>`): the digest recognises hiring
/// postings by it.
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
    Ok(new_items(ctx, postings.collect()))
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
    use crate::trends::period::Period;
    use crate::trends::testutil::{ctx, Fixtures, Paged};

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
        {"objectID": "904", "title": "Ask HN: Who is hiring? (September 2026)", "created_at_i": 1788274800, "num_comments": 1},
        {"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297, "num_comments": 3},
        {"objectID": "903", "title": "Ask HN: Who is hiring? (October 2026)", "created_at_i": 1790866927, "num_comments": 2},
        {"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297, "num_comments": 3}
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
    fn ctx_with(fx: &Fixtures, known: &[&str], limits: impl FnOnce(&mut crate::trends::CollectLimits)) -> Ctx {
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

        let (stories, cut) = period_stories(&ctx, Instant::now()).await.unwrap();
        assert_eq!((stories.len(), cut), (2, 0), "the answer has no nbPages: one page, nothing cut");
        let hit = &fx.hits_on("/hn/search")[0];
        assert!(hit.target.contains("tags=front_page") && hit.target.contains("hitsPerPage=30"), "{}", hit.target);
        let by_date = &fx.hits_on("/hn/search_by_date")[0];
        assert!(by_date.target.contains("hitsPerPage=100") && by_date.target.contains("page=0"), "{}", by_date.target);
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

        let posts = hiring_posts(&ctx, &HashSet::new(), Instant::now()).await.unwrap().items;
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
    async fn the_number_of_hiring_threads_is_capped_and_known_postings_are_skipped() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);

        let two = ctx_with(&fx, &[], |l| l.hiring_threads = 2);
        let found = hiring_posts(&two, &HashSet::new(), Instant::now()).await.unwrap();
        assert_eq!(found.items.len(), 4);
        assert_eq!(found.threads_left, 0, "a thread outside the window is not left unread");
        assert!(fx.hits_on("/hn/items/904").is_empty(), "the third newest thread is not read");

        // A posting that is stored is skipped; every other one is kept.
        let known = ctx_with(&fx, &["https://news.ycombinator.com/item?id=7001"], |_| {});
        let posts = hiring_posts(&known, &HashSet::new(), Instant::now()).await.unwrap().items;
        assert_eq!(
            urls(&posts),
            [
                "https://news.ycombinator.com/item?id=7005",
                "https://news.ycombinator.com/item?id=8001",
                "https://news.ycombinator.com/item?id=8002",
                "https://news.ycombinator.com/item?id=9001"
            ]
        );
    }

    #[tokio::test]
    async fn a_year_reads_more_hiring_threads_than_a_month() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let december = r#"{"id": 901, "children": [{"id": 6001, "created_at_i": 1796300100, "text": "Wayne | Security Engineer | Gotham | Rust, Linux, eBPF, Kubernetes", "children": []}]}"#;
        fx.route("/hn/items/901", 200, "application/json", december);

        let mut month = ctx_with(&fx, &[], |l| l.hiring_threads = 2);
        month.period = Period::parse("2026-M11").unwrap();
        let posts = hiring_posts(&month, &HashSet::new(), Instant::now()).await.unwrap().items;
        assert_eq!(posts.len(), 4, "a month reads `hiring_threads` threads: November and October");
        assert!(fx.hits_on("/hn/items/901").is_empty(), "December starts after the month ends");

        // The same limits, a year: 2 * 4 threads, so the window reaches September and December's
        // thread (it starts before the year ends) is in it.
        let mut year = ctx_with(&fx, &[], |l| l.hiring_threads = 2);
        year.period = Period::parse("2026").unwrap();
        let posts = hiring_posts(&year, &HashSet::new(), Instant::now()).await.unwrap().items;
        assert_eq!(posts.len(), 6, "December, November, October and September");
        let lists = fx.hits_on("/hn/search_by_date");
        assert!(lists[0].target.contains("hitsPerPage=12"), "2 threads, the smallest list: {}", lists[0].target);
        assert!(lists[1].target.contains("hitsPerPage=27"), "8 threads: {}", lists[1].target);

        // The default: 6 threads a month, 24 a year (75 hits: three listed a month, and slack).
        let mut default_year = ctx(&fx, &[]);
        default_year.period = Period::parse("2026").unwrap();
        hiring_posts(&default_year, &HashSet::new(), Instant::now()).await.unwrap();
        let lists = fx.hits_on("/hn/search_by_date");
        assert!(lists[2].target.contains("hitsPerPage=75"), "{}", lists[2].target);
    }

    #[tokio::test]
    async fn a_thread_with_hundreds_of_postings_is_kept_whole() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let comments: Vec<String> = (0..450)
            .map(|n| format!(r#"{{"id": {}, "created_at_i": 1793635303, "text": "Company {n} | Backend Engineer | REMOTE | Rust, Go, Kafka, Postgres", "children": []}}"#, 20_000 + n))
            .collect();
        fx.route("/hn/items/902", 200, "application/json", format!(r#"{{"id": 902, "children": [{}]}}"#, comments.join(",")));

        let found = hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap();
        assert_eq!(found.items.len(), 450 + 3, "all 450 of November, and October's two and September's one");
        assert_eq!(found.cut, 0);
        assert_eq!(found.items[449].url, "https://news.ycombinator.com/item?id=20449", "the last posting is there, in thread order");
    }

    #[tokio::test]
    async fn no_further_thread_starts_once_the_time_is_spent() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let spent = ctx_with(&fx, &[], |l| l.source_timeout = Duration::ZERO);
        let posts = hiring_posts(&spent, &HashSet::new(), Instant::now()).await.unwrap();
        assert!(!posts.items.is_empty(), "the newest thread is always read");
        assert!(fx.hits_on("/hn/items/903").is_empty(), "the older threads are left for a later run");
        assert_eq!((posts.cut, posts.threads_left), (0, 2), "October and September were left unread, and that is counted");
        assert!(posts.settled.is_empty(), "a thread not read is not settled: {:?}", posts.settled);
    }

    /// Settled keys, as an earlier run saved them.
    fn keys(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn a_settled_key_names_the_thread_and_its_comment_count() {
        assert_eq!(settled_key("903", 412), "903@412");
        assert_eq!(settled_thread("903@412"), Some(903));
        for bad in ["903", "903@", "@412", "abc@1", "903@x", ""] {
            assert_eq!(settled_thread(bad), None, "{bad:?}");
        }
    }

    #[tokio::test]
    async fn settled_older_threads_are_not_downloaded_again_and_the_newest_always_is() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        // October is settled; November is listed too, but it is the newest thread.
        let ctx = ctx(&fx, &[]);

        let posts = hiring_posts(&ctx, &keys(&["902@3", "903@2"]), Instant::now()).await.unwrap();
        assert!(fx.hits_on("/hn/items/903").is_empty(), "a settled older thread is not requested");
        assert_eq!(fx.hits_on("/hn/items/902").len(), 1, "the newest thread is read even when listed as settled");
        assert_eq!(fx.hits_on("/hn/items/904").len(), 1, "September is still in the window and not settled yet");
        assert_eq!(
            urls(&posts.items),
            ["https://news.ycombinator.com/item?id=7001", "https://news.ycombinator.com/item?id=7005", "https://news.ycombinator.com/item?id=9001"]
        );
        assert_eq!(posts.settled, ["904@1"], "September is now read in full; the newest never is");

        // A settled thread keeps its place in the window: two threads are November and October,
        // and September stays out.
        let two = ctx_with(&fx, &[], |l| l.hiring_threads = 2);
        let posts = hiring_posts(&two, &keys(&["903@2"]), Instant::now()).await.unwrap();
        assert_eq!(fx.hits_on("/hn/items/904").len(), 1, "September is not read");
        assert_eq!(posts.items.len(), 2);
        assert!(posts.settled.is_empty());
    }

    #[tokio::test]
    async fn a_settled_thread_with_new_comments_is_read_again() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        // October was settled with one comment; it has two now (a late posting).
        let posts = hiring_posts(&ctx(&fx, &[]), &keys(&["903@1", "904@1"]), Instant::now()).await.unwrap();
        assert_eq!(fx.hits_on("/hn/items/903").len(), 1, "October changed: read again");
        assert!(fx.hits_on("/hn/items/904").is_empty(), "September did not");
        assert_eq!(posts.settled, ["903@2"], "settled again with its new count");
    }

    #[tokio::test]
    async fn only_older_threads_read_in_full_are_settled() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let all = hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap();
        assert_eq!(all.settled, ["903@2", "904@1"], "every older thread, newest first; not the newest");

        // A thread is read in full whether or not its postings are new: one of October's is stored.
        let known = ctx(&fx, &["https://news.ycombinator.com/item?id=8001"]);
        assert_eq!(hiring_posts(&known, &HashSet::new(), Instant::now()).await.unwrap().settled, ["903@2", "904@1"]);
        // A thread of hundreds of postings is settled too: nothing was cut.
        let comments: Vec<String> = (0..450)
            .map(|n| format!(r#"{{"id": {}, "created_at_i": 1790867000, "text": "Company {n} | Backend Engineer | REMOTE | Rust, Go, Kafka", "children": []}}"#, 30_000 + n))
            .collect();
        fx.route("/hn/items/903", 200, "application/json", format!(r#"{{"id": 903, "children": [{}]}}"#, comments.join(",")));
        assert_eq!(hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap().settled, ["903@2", "904@1"]);
        fx.route("/hn/items/903", 200, "application/json", THREAD_OCT);

        // A thread that fails to load is not settled.
        fx.route("/hn/items/903", 500, "text/plain", "boom");
        let failed = hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap();
        assert_eq!(failed.settled, ["904@1"]);
        // A thread listed without a comment count is never settled.
        fx.route("/hn/items/903", 200, "application/json", r#"{"id": 903, "children": []}"#);
        let list = r#"{"hits": [
            {"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297, "num_comments": 3},
            {"objectID": "903", "title": "Ask HN: Who is hiring? (October 2026)", "created_at_i": 1790866927}]}"#;
        fx.route("/hn/search_by_date", 200, "application/json", list);
        assert!(hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap().settled.is_empty());
    }

    #[tokio::test]
    async fn a_second_run_reads_nothing_new() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        let first = hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap().items;
        assert_eq!(first.len(), 5);

        let known: Vec<&str> = urls(&first);
        let again = hiring_posts(&ctx(&fx, &known), &HashSet::new(), Instant::now()).await.unwrap().items;
        assert!(again.is_empty(), "every posting is already stored: {:?}", urls(&again));

        // A new posting in the newest thread is all a later run adds.
        fx.route("/hn/items/902", 200, "application/json", THREAD.replace(
            r#"{"id": 7003, "created_at_i": 1793635400, "text": "short reply""#,
            r#"{"id": 7006, "created_at_i": 1793700000, "text": "Vandelay | Rust Engineer | Singapore | Rust, Tokio, Postgres""#,
        ));
        let later = hiring_posts(&ctx(&fx, &known), &HashSet::new(), Instant::now()).await.unwrap().items;
        assert_eq!(urls(&later), ["https://news.ycombinator.com/item?id=7006"]);
    }

    #[tokio::test]
    async fn a_thread_that_fails_is_left_out_and_all_failing_is_an_error() {
        let fx = Fixtures::start().await;
        hiring_fixtures(&fx);
        fx.route("/hn/items/903", 500, "text/plain", "boom");
        let posts = hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap().items;
        assert_eq!(posts.len(), 3, "November and September are kept: {:?}", urls(&posts));

        fx.route("/hn/items/902", 500, "text/plain", "boom");
        fx.route("/hn/items/904", 503, "text/plain", "down");
        let err = hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 500") || err.contains("HTTP 503"), "{err}");

        // No hiring thread before the end: nothing, not an error.
        fx.route("/hn/search_by_date", 200, "application/json", r#"{"hits": []}"#);
        assert!(hiring_posts(&ctx(&fx, &[]), &HashSet::new(), Instant::now()).await.unwrap().items.is_empty());
    }

    /// One answer of the period's search as Algolia pages it: `n` stories from id `first`, of
    /// `hits` over `pages` pages in all (`hits` 0 leaves `nbHits` at 0, as if the answer lacked it).
    fn stories_page(first: u32, n: u32, pages: usize, hits: usize) -> String {
        let stories: Vec<String> = (first..first + n)
            .map(|id| format!(r#"{{"objectID": "{id}", "title": "Story {id}", "url": "https://s.example.com/{id}", "points": 200, "num_comments": 5, "created_at_i": 1794300000}}"#))
            .collect();
        format!(r#"{{"hits": [{}], "nbHits": {hits}, "nbPages": {pages}, "hitsPerPage": {n}}}"#, stories.join(","))
    }

    /// `count` answers of two stories each, the whole result being `pages` pages of `hits` hits.
    fn story_pages(count: u32, pages: usize, hits: usize) -> Vec<String> {
        (0..count).map(|p| stories_page(p * 2, 2, pages, hits)).collect()
    }

    /// A context whose Hacker News endpoint is the paged server, with `limits` changed.
    async fn paged_ctx(paged: &Paged, limits: impl FnOnce(&mut crate::trends::CollectLimits)) -> Ctx {
        let fx = Fixtures::start().await;
        let mut c = ctx_with(&fx, &[], limits);
        let mut config = (*c.config).clone();
        config.endpoints.hackernews = format!("{}/hn", paged.base());
        c.config = Arc::new(config);
        c
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.title.as_str()).collect()
    }

    #[tokio::test]
    async fn the_periods_stories_are_read_page_by_page_to_the_last_page() {
        // Four pages exist, the first answer says there are three: the fourth is never asked for.
        let paged = Paged::start(story_pages(4, 3, 6)).await;
        let c = paged_ctx(&paged, |_| {}).await;

        let (stories, cut) = period_stories(&c, Instant::now()).await.unwrap();
        assert_eq!(titles(&stories), ["Story 0", "Story 1", "Story 2", "Story 3", "Story 4", "Story 5"]);
        assert_eq!(cut, 0, "the last page was reached");
        let targets = paged.targets();
        assert_eq!(targets.len(), 3, "{targets:?}");
        for (n, target) in targets.iter().enumerate() {
            assert!(target.starts_with("/hn/search_by_date?") && target.ends_with(&format!("page={n}")), "{target}");
            assert!(target.contains("hitsPerPage=100") && target.contains("points%3E%3D100"), "{target}");
        }

        // An answer without nbPages is a single page.
        let single = Paged::start(vec![stories_page(0, 2, 0, 0), stories_page(2, 2, 0, 0)]).await;
        let (stories, cut) = period_stories(&paged_ctx(&single, |_| {}).await, Instant::now()).await.unwrap();
        assert_eq!((stories.len(), cut, single.targets().len()), (2, 0, 1));

        // The API serves only the first pages of a large result: its last page is not the end.
        let capped = Paged::start(story_pages(3, 3, 50)).await;
        let (stories, cut) = period_stories(&paged_ctx(&capped, |_| {}).await, Instant::now()).await.unwrap();
        assert_eq!((stories.len(), cut), (6, 44), "50 hits, 6 served");
    }

    #[tokio::test]
    async fn the_page_cap_stops_paging_and_what_is_left_is_counted() {
        let paged = Paged::start(story_pages(5, 5, 10)).await;
        let c = paged_ctx(&paged, |l| l.hn_story_pages = 2).await;

        let (stories, cut) = period_stories(&c, Instant::now()).await.unwrap();
        assert_eq!(titles(&stories), ["Story 0", "Story 1", "Story 2", "Story 3"]);
        assert_eq!(cut, 6, "10 hits, 4 read");
        assert_eq!(paged.targets().len(), 2, "no third page is asked for");

        // An answer that does not say how many hits there are: one cut marker.
        let unknown = Paged::start(story_pages(5, 5, 0)).await;
        let (stories, cut) = period_stories(&paged_ctx(&unknown, |l| l.hn_story_pages = 2).await, Instant::now()).await.unwrap();
        assert_eq!((stories.len(), cut), (4, 1));

        // A cap of 0 still reads the first page.
        let (stories, _) = period_stories(&paged_ctx(&paged, |l| l.hn_story_pages = 0).await, Instant::now()).await.unwrap();
        assert_eq!(stories.len(), 2);
    }

    #[tokio::test]
    async fn the_time_budget_stops_paging_after_the_first_page() {
        let paged = Paged::start(story_pages(5, 5, 10)).await;
        let spent = paged_ctx(&paged, |l| l.source_timeout = Duration::ZERO).await;
        let (stories, cut) = period_stories(&spent, Instant::now()).await.unwrap();
        assert_eq!(titles(&stories), ["Story 0", "Story 1"], "the first page is always read, what is in hand is kept");
        assert_eq!(cut, 8);
        assert_eq!(paged.targets().len(), 1);
    }

    #[tokio::test]
    async fn a_failing_later_page_keeps_the_earlier_pages_and_a_failing_first_page_is_an_error() {
        // Only the first page is served; the answer says there are three.
        let paged = Paged::start(story_pages(1, 3, 6)).await;
        let (stories, cut) = period_stories(&paged_ctx(&paged, |_| {}).await, Instant::now()).await.unwrap();
        assert_eq!((stories.len(), cut), (2, 4), "page 1 is a 404: the first page is kept and the other 4 hits are counted");

        let none = Paged::start(vec![]).await;
        let err = period_stories(&paged_ctx(&none, |_| {}).await, Instant::now()).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 404"), "{err}");
    }

    #[tokio::test]
    async fn collect_keeps_every_story_beyond_max_items_and_reports_the_cut() {
        // The same pages answer the front page and the hiring list too (no `page` means page 0):
        // stories 0 and 1 are on the front page and on page 0 of the period.
        let paged = Paged::start(story_pages(3, 3, 6)).await;
        let c = paged_ctx(&paged, |l| {
            l.hn_story_pages = 2;
            l.max_items = 1;
        })
        .await;

        let hn = collect(&c, &HashSet::new()).await.unwrap();
        assert_eq!(titles(&hn.items), ["Story 0", "Story 1", "Story 2", "Story 3"], "max_items does not cap the stories; the page cap does");
        assert_eq!(hn.cut, 2, "6 hits, 4 read");
        assert!(hn.settled.is_empty());
    }

    #[tokio::test]
    async fn collect_merges_dedups_and_survives_one_failing_part() {
        let fx = Fixtures::start().await;
        fixtures(&fx);
        // `search_by_date` serves both the period's stories and the thread list: give it a body that
        // is valid for both shapes (hits with a title and no hiring thread).
        let ctx = ctx(&fx, &["https://pg.example.org/19"]);
        let items = collect(&ctx, &HashSet::new()).await.unwrap().items;
        let urls: Vec<_> = items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://blog.example.com/rust-2", "https://news.ycombinator.com/item?id=2", "https://news.ycombinator.com/item?id=4"],
            "the story that is in both lists once; the known one dropped"
        );

        // The front page fails, the rest is kept.
        fx.route("/hn/search", 500, "text/plain", "boom");
        let items = collect(&ctx, &HashSet::new()).await.unwrap().items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://blog.example.com/rust-2");

        // Everything fails: an error, not an empty success.
        fx.route("/hn/search_by_date", 503, "text/plain", "down");
        let err = collect(&ctx, &HashSet::new()).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 500") || err.contains("HTTP 503"), "{err}");
    }
}
