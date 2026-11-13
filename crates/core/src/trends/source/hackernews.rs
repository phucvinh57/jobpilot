//! Hacker News through the Algolia API: the front page, the period's most upvoted stories, and the
//! newest "Ask HN: Who is hiring?" thread (one item per posting).
//!
//! A story's item is the story itself (its link, or the HN thread for text posts): the title and
//! the numbers are the text; the linked article is not fetched.

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use super::{join, normalize_url, take_new, Ctx, Item};
use crate::trends::text::{first_line, html_to_text};

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
    let mut items = Vec::new();
    let mut failures = Vec::new();
    for part in [front_page(ctx).await, period_stories(ctx).await] {
        match part {
            Ok(found) => items.extend(found),
            Err(e) => failures.push(e),
        }
    }
    let mut items = take_new(ctx, items, ctx.limits().max_items);
    let hiring = hiring_posts(ctx).await;
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

/// The newest "Who is hiring" thread that started before the period ended, one item per posting.
async fn hiring_posts(ctx: &Ctx) -> Result<Vec<Item>> {
    let list = api_url(ctx, "search_by_date", &[("tags", "story,author_whoishiring"), ("hitsPerPage", "12")])?;
    let threads: Search = ctx.http.get(&list, &[]).await?.json("Hacker News")?;
    let end = ctx.period.end().timestamp();
    let Some(thread) = threads
        .hits
        .into_iter()
        .filter(|h| h.title.as_deref().is_some_and(|t| t.to_lowercase().contains("who is hiring")))
        .filter(|h| h.created_at_i.is_some_and(|t| t < end))
        .max_by_key(|h| h.created_at_i)
    else {
        return Ok(Vec::new());
    };
    let title = thread.title.unwrap_or_default();
    let full: Thread = ctx
        .http
        .get(&join(&ctx.endpoints().hackernews, &format!("items/{}", thread.object_id)), &[])
        .await?
        .json("Hacker News")
        .context("reading the hiring thread")?;

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

    use super::*;
    use crate::trends::testutil::{ctx, Fixtures};

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
        {"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297},
        {"objectID": "903", "title": "Ask HN: Who is hiring? (October 2026)", "created_at_i": 1790866927}
    ]}"#;

    const THREAD: &str = r#"{"id": 902, "title": "Ask HN: Who is hiring? (November 2026)", "children": [
        {"id": 7001, "created_at_i": 1793635303, "text": "Acme | Senior Rust Engineer | REMOTE | &#x24;150k<p>We build robots in Rust &amp; Go. Apply at <a href=\"https:&#x2F;&#x2F;acme.test&#x2F;jobs\">https:&#x2F;&#x2F;acme.test&#x2F;jobs</a>", "children": [{"id": 7002, "text": "reply", "children": []}]},
        {"id": 7003, "created_at_i": 1793635400, "text": "short reply", "children": []},
        {"id": 7004, "created_at_i": 1793635500, "text": null, "children": []},
        {"id": 7005, "created_at_i": 1793635600, "text": "Globex | Data Engineer | Berlin, ONSITE | Python, Spark, Kafka, dbt, Airflow, Snowflake", "children": []}
    ]}"#;

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
    async fn hiring_picks_the_thread_before_the_period_end_and_one_item_per_posting() {
        let fx = Fixtures::start().await;
        // The thread list and the period's stories share /search_by_date: answer by tag in one route.
        fx.route("/hn/search_by_date", 200, "application/json", THREADS);
        fx.route("/hn/items/902", 200, "application/json", THREAD);
        let ctx = ctx(&fx, &[]);

        let posts = hiring_posts(&ctx).await.unwrap();
        // December's thread starts after 2026-W46 and "Who wants to be hired" is another thread.
        assert!(fx.hits_on("/hn/items/902").len() == 1 && fx.hits_on("/hn/items/901").is_empty());
        assert_eq!(posts.len(), 2, "replies, empty and deleted comments are not postings");
        assert_eq!(posts[0].url, "https://news.ycombinator.com/item?id=7001");
        assert_eq!(posts[0].title, "Acme | Senior Rust Engineer | REMOTE | $150k");
        assert!(posts[0].text.starts_with("# Hacker News, Ask HN: Who is hiring? (November 2026)\n\nAcme |"), "{}", posts[0].text);
        assert!(posts[0].text.contains("We build robots in Rust & Go. Apply at https://acme.test/jobs"), "{}", posts[0].text);
        assert_eq!(posts[1].url, "https://news.ycombinator.com/item?id=7005");

        // Known postings are skipped, and the cap counts only new ones.
        let known = ctx_with_known(&fx, &["https://news.ycombinator.com/item?id=7001"], 1);
        let posts = hiring_posts(&known).await.unwrap();
        assert_eq!(posts.iter().map(|p| p.url.as_str()).collect::<Vec<_>>(), ["https://news.ycombinator.com/item?id=7005"]);
    }

    fn ctx_with_known(fx: &Fixtures, known: &[&str], hiring_posts: usize) -> Ctx {
        let mut c = ctx(fx, known);
        let mut config = (*c.config).clone();
        config.limits.hiring_posts = hiring_posts;
        c.config = Arc::new(config);
        c
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
