//! A person the user follows (a founder, a CEO, a researcher): what they said in interviews, talks
//! and conversations, as items the digest can quote and link.
//!
//! Two places are read, both read-only:
//!
//! 1. **Stories that name them.** The Hacker News search (Algolia) for stories whose title carries
//!    the person's name, newest first, from a month before the period to its end. The story's link
//!    is the interview, the talk or the article; its page is fetched (up to
//!    `person_fetches` per run) so the digest reads the words, not only the headline. A page that
//!    cannot be read (a video, a paywall) leaves an item with the title alone: the link is kept.
//! 2. **An address of their own**, when the user gave one: a feed (a podcast, a blog, a channel)
//!    or a page (a transcript). Every entry of the feed counts as theirs.
//!
//! An item's address is always the conversation's own address, never a search page: it is what
//! the report links. A story is *about* the person, which is not the same as the person speaking;
//! the digest therefore quotes the item's words for every statement it attributes
//! (`digest::cite`), and the reader can open the link.

use std::collections::HashSet;

use anyhow::Result;
use chrono::Duration;

use super::page::{self, fetch_text, make_item};
use super::{hackernews, rss, take_new, Ctx, Item};
use crate::trends::http::{public_http_url, FetchError};

/// Stories this long before the period start are still read (an interview from last month is
/// still what the person thinks).
const BACKLOG_DAYS: i64 = 30;

/// The new items about `name`, and how many more a cap cut. A part that fails (the search, the
/// person's own address) is left out; the adapter fails only when nothing was read.
pub async fn collect(ctx: &Ctx, name: &str, url: &str) -> Result<(Vec<Item>, usize)> {
    let mut items = Vec::new();
    let mut cut = 0;
    let mut failures = Vec::new();
    if !url.is_empty() {
        match own_address(ctx, url).await {
            Ok((found, unread)) => {
                items.extend(found);
                cut += unread;
            }
            Err(e) => failures.push(e),
        }
    }
    match stories(ctx, name).await {
        Ok(found) => items.extend(found),
        Err(e) => failures.push(e),
    }
    let (items, more) = take_new(ctx, items, ctx.limits().max_items);
    cut += more;
    match failures.into_iter().next() {
        Some(first) if items.is_empty() => Err(first),
        Some(_) => {
            tracing::warn!(kept = items.len(), "one place of a followed person could not be read; the other is kept");
            Ok((items, cut))
        }
        None => Ok((items, cut)),
    }
}

/// The person's own address: a feed, else a page. A feed that cannot be fetched is an error (the
/// page would fail the same way); an address that is not a feed is read as a page.
async fn own_address(ctx: &Ctx, url: &str) -> Result<(Vec<Item>, usize)> {
    match rss::collect(ctx, url).await {
        Ok(found) => Ok(found),
        Err(e) if e.chain().any(|cause| cause.downcast_ref::<FetchError>().is_some()) => Err(e),
        Err(_) => Ok((page::collect(ctx, url).await?, 0)),
    }
}

/// The stories naming the person, each with the text of the page it links when that can be read.
async fn stories(ctx: &Ctx, name: &str) -> Result<Vec<Item>> {
    let l = ctx.limits();
    let since = ctx.period.start() - Duration::days(BACKLOG_DAYS);
    let found = hackernews::stories_naming(ctx, name, since).await?;
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut fetched = 0;
    for story in found {
        let mut item = story.item;
        if ctx.is_known(&item.url) || !seen.insert(item.url.clone()) {
            continue;
        }
        if story.linked && fetched < l.person_fetches && public_http_url(&item.url, l.allow_private_hosts).is_ok() {
            fetched += 1;
            match fetch_text(ctx, &item.url).await {
                Ok(page) => item = make_item(item.url, Some(&item.title), item.published_at, page.text),
                // The headline and the link are still worth keeping.
                Err(_) => tracing::debug!("a story's page could not be read; its title is kept"),
            }
        }
        items.push(item);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::trends::testutil::{ctx, Fixtures};

    fn article(heading: &str, sentence: &str) -> String {
        format!("<html><head><title>{heading}</title></head><body><article><h1>{heading}</h1>{}</article></body></html>", format!("<p>{sentence}</p>").repeat(6))
    }

    fn hit(id: &str, title: &str, url: Option<String>) -> serde_json::Value {
        json!({"objectID": id, "title": title, "url": url, "points": 120, "num_comments": 40, "created_at_i": 1794312000})
    }

    #[tokio::test]
    async fn stories_naming_the_person_are_read_with_the_page_they_link() {
        let fx = Fixtures::start().await;
        let hits = json!({"hits": [
            hit("1", "Ada Example: every company will run an AI factory", Some(format!("{}/interview", fx.base()))),
            hit("2", "Ada Example's keynote (video)", Some(format!("{}/video", fx.base()))),
            hit("3", "Ask HN: what does Ada Example mean by AI factory?", None),
            hit("4", "Example Corp earnings beat estimates", Some(format!("{}/earnings", fx.base()))),
            hit("5", "Ada Example on robots", Some(format!("{}/known", fx.base()))),
        ]});
        fx.route("/hn/search_by_date", 200, "application/json", hits.to_string());
        fx.route("/interview", 200, "text/html", article("Interview", "I think every company will run an AI factory within five years, said Ada Example."));
        fx.route("/video", 200, "text/html", "<html><body><div id=player></div></body></html>");
        let known = format!("{}/known", fx.base());
        let ctx = ctx(&fx, &[&known]);

        let (items, cut) = collect(&ctx, "Ada Example", "").await.unwrap();
        assert_eq!(cut, 0);
        let urls: Vec<&str> = items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(
            urls,
            [format!("{}/interview", fx.base()), format!("{}/video", fx.base()), "https://news.ycombinator.com/item?id=3".to_string()],
            "the story that does not name the person and the known one are left out"
        );
        assert_eq!(items[0].title, "Ada Example: every company will run an AI factory");
        assert!(items[0].text.contains("every company will run an AI factory within five years"), "the page was read: {}", items[0].text);
        assert_eq!(items[0].published_at.unwrap().timestamp(), 1794312000);
        assert!(items[1].text.contains("on Hacker News"), "a page without text keeps the story: {}", items[1].text);

        let search = fx.hits().into_iter().find(|h| h.path() == "/hn/search_by_date").unwrap();
        let query = search.target.as_str();
        assert!(query.contains("query=%22Ada+Example%22"), "{query}");
        assert!(query.contains("restrictSearchableAttributes=title") && query.contains("tags=story"), "{query}");
        assert!(query.contains("points%3E%3D10"), "{query}");
        let paths: Vec<String> = fx.hits().iter().map(|h| h.path().to_string()).collect();
        assert_eq!(paths, ["/hn/search_by_date", "/interview", "/video"], "the HN thread and the known page are not fetched");
    }

    #[tokio::test]
    async fn the_page_fetches_are_capped_and_the_rest_keep_their_title() {
        let fx = Fixtures::start().await;
        let hits: Vec<_> = (1..=3).map(|n| hit(&n.to_string(), &format!("Ada Example talk {n}"), Some(format!("{}/talk/{n}", fx.base())))).collect();
        fx.route("/hn/search_by_date", 200, "application/json", json!({"hits": hits}).to_string());
        for n in 1..=3 {
            fx.route(&format!("/talk/{n}"), 200, "text/html", article("Talk", "Software engineers will direct agents more than they write code."));
        }
        let mut c = ctx(&fx, &[]);
        let mut config = (*c.config).clone();
        config.limits.person_fetches = 1;
        c.config = std::sync::Arc::new(config);

        let (items, _) = collect(&c, "ada example", "").await.unwrap();
        assert_eq!(items.len(), 3, "the name matches whatever its case");
        assert!(items[0].text.contains("direct agents"));
        assert!(!items[1].text.contains("direct agents") && !items[2].text.contains("direct agents"));
    }

    #[tokio::test]
    async fn an_own_address_is_read_as_a_feed_or_as_a_page() {
        let fx = Fixtures::start().await;
        fx.route("/hn/search_by_date", 200, "application/json", json!({"hits": []}).to_string());
        let body = "The next decade belongs to people who can combine domain knowledge with AI tools. ".repeat(6);
        let feed = format!(
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Talks</title><item><title>Fireside chat</title><link>{}/chat</link><pubDate>Tue, 10 Nov 2026 12:00:00 GMT</pubDate><description>{body}</description></item></channel></rss>"#,
            fx.base()
        );
        fx.route("/feed.xml", 200, "application/rss+xml", feed);
        fx.route("/transcript", 200, "text/html", article("Transcript", "We will need more electricians and plumbers to build the data centres."));
        let ctx = ctx(&fx, &[]);

        let (items, _) = collect(&ctx, "Ada Example", &format!("{}/feed.xml", fx.base())).await.unwrap();
        assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Fireside chat"]);
        assert_eq!(items[0].url, format!("{}/chat", fx.base()));

        let (items, _) = collect(&ctx, "Ada Example", &format!("{}/transcript", fx.base())).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, format!("{}/transcript", fx.base()));
        assert!(items[0].text.contains("electricians and plumbers"));
    }

    #[tokio::test]
    async fn one_failing_place_keeps_the_other_and_nothing_read_is_an_error() {
        let fx = Fixtures::start().await;
        fx.route("/hn/search_by_date", 200, "application/json", json!({"hits": [hit("9", "Ada Example on chips", None)]}).to_string());
        fx.route("/gone.xml", 404, "text/plain", "x");
        let (items, _) = collect(&ctx(&fx, &[]), "Ada Example", &format!("{}/gone.xml", fx.base())).await.unwrap();
        assert_eq!(items.len(), 1, "the search still gave a story");

        let fx = Fixtures::start().await;
        fx.route("/hn/search_by_date", 500, "text/plain", "boom");
        assert!(collect(&ctx(&fx, &[]), "Ada Example", "").await.is_err());
    }
}
