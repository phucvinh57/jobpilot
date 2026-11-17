//! RSS 2.0 / RSS 1.0 (RDF) / Atom feeds the user adds (engineering blogs, newsletters).
//!
//! The parser is deliberately small (`quick-xml`, the fields the digest needs): per entry the
//! title, link, date and body. A feed that carries only teasers has the entry's page fetched too,
//! within the per-run cap and the per-host delay.

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::page::{fetch_text, make_item};
use super::{normalize_url, take_new, Ctx, Item};
use crate::trends::http::public_http_url;
use crate::trends::text::{cap, decode_entities, html_to_text, parse_date};

/// A feed body shorter than this is a teaser; the entry's own page is fetched instead.
const TEASER_CHARS: usize = 300;
/// Entries older than this before the period start are skipped (feeds keep years of posts).
const BACKLOG_DAYS: i64 = 30;
const ACCEPT: &str = "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/xml;q=0.8, */*;q=0.5";

/// One feed entry as the feed gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    /// As written in the feed (may be relative).
    pub link: Option<String>,
    pub published: Option<DateTime<Utc>>,
    /// Content, else summary; HTML or text as the feed wrote it.
    pub body: String,
}

#[derive(Default)]
struct Draft {
    title: String,
    link: Option<String>,
    atom_link: Option<String>,
    guid: Option<String>,
    summary: String,
    content: String,
    published: Option<DateTime<Utc>>,
    updated: Option<DateTime<Utc>>,
}

impl Draft {
    /// What a child element of the entry said.
    fn set(&mut self, field: &str, text: &str) {
        let text = text.trim();
        match field {
            "title" => self.title = text.split_whitespace().collect::<Vec<_>>().join(" "),
            "link" if !text.is_empty() && self.link.is_none() => self.link = Some(text.to_string()),
            "guid" | "id" if !text.is_empty() => self.guid = Some(text.to_string()),
            "description" | "summary" => self.summary = text.to_string(),
            "content:encoded" | "content" => self.content = text.to_string(),
            "pubDate" | "published" | "dc:date" | "date" => self.published = self.published.or_else(|| parse_date(text)),
            "updated" => self.updated = self.updated.or_else(|| parse_date(text)),
            _ => {}
        }
    }

    fn finish(self) -> Entry {
        // RSS: <link>; Atom: <link href> (alternate); else a guid that is itself an address.
        let guid_link = self.guid.filter(|g| g.starts_with("http://") || g.starts_with("https://"));
        let body = if self.content.is_empty() { self.summary } else { self.content };
        Entry {
            title: self.title,
            link: self.link.or(self.atom_link).or(guid_link),
            published: self.published.or(self.updated),
            body,
        }
    }
}

/// The `href` of an Atom `<link>` that points at the entry's page (`rel` missing or `alternate`).
fn atom_href(e: &BytesStart) -> Option<String> {
    let (mut href, mut rel) = (None, None);
    for attr in e.attributes().with_checks(false).flatten() {
        let value = attr.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned());
        match attr.key.as_ref() {
            "href" => href = value,
            "rel" => rel = value,
            _ => {}
        }
    }
    href.filter(|_| matches!(rel.as_deref(), None | Some("alternate")))
}

fn name_of(e: &BytesStart) -> String {
    e.name().as_ref().to_string()
}

/// The entries of an RSS or Atom document, in document order.
pub fn parse_feed(xml: &str) -> Result<Vec<Entry>> {
    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();
    let mut seen_root = false;
    // Inside an entry: the draft, how many elements are open below the entry, the direct child
    // being read, and its text (the text of everything inside it, tags dropped).
    let mut draft: Option<Draft> = None;
    let mut nest = 0usize;
    let mut field = String::new();
    let mut text = String::new();
    loop {
        let event = reader.read_event().map_err(|e| anyhow::anyhow!("not a readable feed: {e} (byte {})", reader.buffer_position()))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = name_of(&e);
                match draft.as_mut() {
                    None => match name.as_str() {
                        "rss" | "feed" | "rdf:RDF" | "RDF" => seen_root = true,
                        "item" | "entry" => {
                            draft = Some(Draft::default());
                            nest = 0;
                        }
                        _ => {}
                    },
                    Some(d) => {
                        nest += 1;
                        if nest == 1 {
                            if name == "link" {
                                d.atom_link = d.atom_link.take().or_else(|| atom_href(&e));
                            }
                            field = name;
                            text.clear();
                        }
                    }
                }
            }
            Event::Empty(e) => {
                if let (Some(d), 0) = (draft.as_mut(), nest) {
                    if name_of(&e) == "link" {
                        d.atom_link = d.atom_link.take().or_else(|| atom_href(&e));
                    }
                }
            }
            Event::End(_) => {
                if let Some(d) = draft.as_mut() {
                    if nest == 0 {
                        entries.push(std::mem::take(d).finish());
                        draft = None;
                    } else {
                        if nest == 1 {
                            d.set(&field, &text);
                        }
                        nest -= 1;
                    }
                }
            }
            Event::Text(t) if draft.is_some() && nest >= 1 => {
                text.push_str(&t.xml10_content());
            }
            Event::CData(c) if draft.is_some() && nest >= 1 => {
                text.push_str(&c.xml10_content());
            }
            Event::GeneralRef(r) if draft.is_some() && nest >= 1 => {
                if let Ok(Some(ch)) = r.resolve_char_ref() {
                    text.push(ch);
                } else {
                    let name = r.xml10_content();
                    // `&amp;` `&lt;` and the HTML names some feeds use without declaring them.
                    text.push_str(&decode_entities(&format!("&{name};")));
                }
            }
            _ => {}
        }
    }
    if !seen_root && entries.is_empty() {
        bail!("the address is not an RSS or Atom feed");
    }
    Ok(entries)
}

/// New entries of the feed at `feed_url` as items.
pub async fn collect(ctx: &Ctx, feed_url: &str) -> Result<Vec<Item>> {
    let l = ctx.limits();
    let resp = ctx.http.get(feed_url, &[("Accept", ACCEPT)]).await?;
    let base = url::Url::parse(&resp.url)?;
    let mut entries = parse_feed(&resp.text())?;
    // Newest first (undated entries keep their place at the end).
    entries.sort_by_key(|e| std::cmp::Reverse(e.published));
    let cutoff = ctx.period.start() - Duration::days(BACKLOG_DAYS);

    let mut items: Vec<Item> = Vec::new();
    let mut fetched = 0;
    for entry in entries {
        if items.len() >= l.max_items {
            break;
        }
        if entry.published.is_some_and(|p| p < cutoff) {
            continue;
        }
        let Some(url) = entry.link.as_deref().and_then(|link| base.join(link).ok()).and_then(|u| normalize_url(u.as_str()).ok()) else {
            continue;
        };
        if ctx.is_known(&url) || items.iter().any(|i| i.url == url) {
            continue;
        }
        let mut body = html_to_text(&entry.body);
        if body.chars().count() < TEASER_CHARS && fetched < l.article_fetches && public_http_url(&url, l.allow_private_hosts).is_ok() {
            fetched += 1;
            match fetch_text(ctx, &url).await {
                Ok(page) => body = page.text,
                // The teaser is still a usable item.
                Err(_) => tracing::debug!("a feed entry's page could not be read; the feed's own text is kept"),
            }
        }
        if body.is_empty() && entry.title.is_empty() {
            continue;
        }
        items.push(make_item(url, Some(&entry.title), entry.published, cap(&body, l.max_text_chars)));
    }
    Ok(take_new(ctx, items, l.max_items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trends::testutil::{ctx, Fixtures};

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:atom="http://www.w3.org/2005/Atom">
<channel>
  <title>Eng Blog</title><link>https://blog.test/</link>
  <atom:link href="https://blog.test/feed.xml" rel="self" type="application/rss+xml"/>
  <item>
    <title>Scaling Postgres &amp; Kafka</title>
    <link>https://blog.test/posts/scaling</link>
    <pubDate>Tue, 10 Nov 2026 12:00:00 GMT</pubDate>
    <description><![CDATA[<p>Short teaser.</p>]]></description>
    <content:encoded><![CDATA[<h1>Scaling</h1><p>We moved to <b>Postgres 19</b> and Kafka.</p><p>Latency dropped 40%.</p><p>The migration took three quarters, covered forty services, and taught the platform team a great deal about backpressure, schema evolution and the real cost of dual writes during a long cutover. Along the way we rewrote the deploy tooling, retired two internal queues and wrote down every lesson for the next team.</p>]]></content:encoded>
    <guid isPermaLink="false">post-1</guid>
  </item>
  <item>
    <title>Teaser only</title>
    <link>/posts/teaser</link>
    <dc:date>2026-11-09T08:00:00Z</dc:date>
    <description>A short note &amp; a link &#8211; &nbsp;read on.</description>
  </item>
  <item>
    <title>Old post</title>
    <link>https://blog.test/posts/old</link>
    <pubDate>Mon, 01 Jan 2024 00:00:00 GMT</pubDate>
    <description>Ancient history.</description>
  </item>
  <item>
    <title>Guid only</title>
    <guid>https://blog.test/posts/guid</guid>
    <description>Body of the guid-only post.</description>
  </item>
  <item><title>No link at all</title><description>nothing to cite</description></item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Blog</title><link href="https://a.test/" rel="alternate"/><link href="https://a.test/atom.xml" rel="self"/>
  <entry>
    <title type="html">Rust &lt;3 embedded</title>
    <link rel="replies" href="https://a.test/replies"/>
    <link rel="alternate" href="https://a.test/rust-embedded"/>
    <id>tag:a.test,2026:1</id>
    <updated>2026-11-11T10:00:00Z</updated>
    <published>2026-11-10T09:30:00+01:00</published>
    <author><name>Ann</name></author>
    <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml"><p>Embedded Rust adoption grew.</p><p>Second paragraph.</p></div></content>
  </entry>
  <entry>
    <title>Updated only</title>
    <link href="https://a.test/updated-only"/>
    <updated>2026-11-08T10:00:00Z</updated>
    <summary type="html">&lt;p&gt;Summary &lt;em&gt;text&lt;/em&gt;&lt;/p&gt;</summary>
  </entry>
</feed>"#;

    const RDF: &str = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <channel rdf:about="https://r.test/"><title>RDF</title><link>https://r.test/</link></channel>
  <item rdf:about="https://r.test/1"><title>RDF item</title><link>https://r.test/1</link><dc:date>2026-11-10T00:00:00Z</dc:date><description>RDF body</description></item>
</rdf:RDF>"#;

    #[test]
    fn rss_items_have_title_link_date_and_the_fuller_body() {
        let entries = parse_feed(RSS).unwrap();
        assert_eq!(entries.len(), 5, "the channel's own title and atom:link are not entries");
        let first = &entries[0];
        assert_eq!(first.title, "Scaling Postgres & Kafka");
        assert_eq!(first.link.as_deref(), Some("https://blog.test/posts/scaling"));
        assert_eq!(first.published.unwrap().timestamp(), 1794312000);
        assert!(first.body.contains("<b>Postgres 19</b>") && !first.body.contains("Short teaser"), "content:encoded beats the teaser: {}", first.body);
        assert_eq!(entries[1].published.unwrap().timestamp(), 1794211200, "dc:date");
        assert_eq!(entries[1].body, "A short note & a link \u{2013}  read on.", "entities, also HTML ones, are resolved");
        assert_eq!(entries[3].link.as_deref(), Some("https://blog.test/posts/guid"), "an address guid stands in for the link");
        assert_eq!(entries[4].link, None);
    }

    #[test]
    fn atom_entries_use_the_alternate_link_and_flatten_xhtml() {
        let entries = parse_feed(ATOM).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Rust <3 embedded");
        assert_eq!(entries[0].link.as_deref(), Some("https://a.test/rust-embedded"));
        assert_eq!(entries[0].published.unwrap().to_rfc3339(), "2026-11-10T08:30:00+00:00", "published wins over updated");
        assert!(entries[0].body.contains("Embedded Rust adoption grew.") && entries[0].body.contains("Second paragraph."), "{}", entries[0].body);
        assert_eq!(entries[1].link.as_deref(), Some("https://a.test/updated-only"));
        assert_eq!(entries[1].published.unwrap().to_rfc3339(), "2026-11-08T10:00:00+00:00", "updated when nothing else");
        assert_eq!(html_to_text(&entries[1].body), "Summary text", "escaped HTML in a summary");
    }

    #[test]
    fn rss_one_point_zero_and_non_feeds() {
        let entries = parse_feed(RDF).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].link.as_deref(), Some("https://r.test/1"));
        assert_eq!(entries[0].published.unwrap().timestamp(), 1794268800);

        assert!(parse_feed("<html><body>hi</body></html>").unwrap_err().to_string().contains("not an RSS or Atom feed"));
        assert!(parse_feed("<rss><channel><item><title>x</title></channel>").is_err(), "mismatched tags");
        assert_eq!(parse_feed("<rss version=\"2.0\"><channel><title>empty</title></channel></rss>").unwrap(), vec![]);
    }

    #[tokio::test]
    async fn collect_skips_old_and_known_entries_and_fetches_pages_for_teasers() {
        let fx = Fixtures::start().await;
        fx.route("/feed.xml", 200, "application/rss+xml", RSS.replace("https://blog.test", fx.base()));
        let page = "<html><head><title>Teaser only</title></head><body><article><h1>Teaser only</h1>".to_string()
            + &"<p>The full article explains how platform teams adopted Rust for services and why Kafka stayed.</p>".repeat(6)
            + "</article></body></html>";
        fx.route("/posts/teaser", 200, "text/html", page);
        let ctx = ctx(&fx, &[&format!("{}/posts/guid", fx.base())]);

        let items = collect(&ctx, &format!("{}/feed.xml", fx.base())).await.unwrap();
        let urls: Vec<_> = items.iter().map(|i| i.url.clone()).collect();
        // Entries: scaling (absolute), teaser (relative to the feed), old (too old), guid (known),
        // no link.
        assert_eq!(urls, [format!("{}/posts/scaling", fx.base()), format!("{}/posts/teaser", fx.base())]);
        assert_eq!(items[0].title, "Scaling Postgres & Kafka");
        assert!(items[0].text.starts_with("# Scaling Postgres & Kafka\n\n"), "{}", items[0].text);
        assert!(items[0].text.contains("We moved to Postgres 19 and Kafka.\n\nLatency dropped 40%."), "{}", items[0].text);
        assert!(items[1].text.contains("The full article explains how platform teams adopted Rust"), "the page replaced the teaser: {}", items[1].text);
        assert_eq!(items[1].published_at.unwrap().timestamp(), 1794211200, "feed date kept");
        // The scaling entry carries its full text: only the teaser's page was fetched.
        assert_eq!(fx.hits().iter().map(|h| h.path().to_string()).collect::<Vec<_>>(), ["/feed.xml", "/posts/teaser"]);
    }

    #[tokio::test]
    async fn a_failing_article_page_keeps_the_teaser_and_a_bad_feed_fails() {
        let fx = Fixtures::start().await;
        fx.route("/feed.xml", 200, "application/rss+xml", RSS.replace("https://blog.test", fx.base()));
        fx.route("/posts/teaser", 500, "text/plain", "boom");
        let items = collect(&ctx(&fx, &[]), &format!("{}/feed.xml", fx.base())).await.unwrap();
        let teaser = items.iter().find(|i| i.title == "Teaser only").unwrap();
        assert!(teaser.text.contains("A short note & a link"), "{}", teaser.text);

        fx.route("/page.html", 200, "text/html", "<html><body>not a feed</body></html>");
        let err = collect(&ctx(&fx, &[]), &format!("{}/page.html", fx.base())).await.unwrap_err().to_string();
        assert!(err.contains("not an RSS or Atom feed"), "{err}");
        fx.route("/gone.xml", 404, "text/plain", "x");
        assert!(collect(&ctx(&fx, &[]), &format!("{}/gone.xml", fx.base())).await.unwrap_err().to_string().contains("HTTP 404"));
    }
}
