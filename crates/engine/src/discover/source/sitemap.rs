//! A careers site's `sitemap.xml`: when neither the careers page nor its JSON-LD gives postings,
//! the posting-shaped addresses in the sitemap are cheaper than rendering the page in a browser.
//!
//! Code picks every address: the sitemap is always `<scheme>://<configured host>/sitemap.xml`; of a
//! sitemap index only a few child sitemaps on the same site whose address names jobs are read; and
//! a `<loc>` is a candidate only when it passes [`same_site`] and [`looks_like_posting`], like a link
//! on the page. A missing or broken sitemap is not an error: the caller moves on to the browser.

use std::cmp::Reverse;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use quick_xml::Reader;
use tracing::info;
use url::Url;

use super::jsonld::{parse_date, parse_datetime};
use super::web::{page_url, same_site, looks_like_posting, title_from_url, Pages, JOB_WORDS};
use super::Candidate;

/// Most child sitemaps of an index that are read.
const MAX_CHILDREN: usize = 3;
/// Most `<loc>` entries kept from one file (the sitemap protocol's own limit).
const MAX_ENTRIES: usize = 50_000;

/// One `<url>` or `<sitemap>` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Entry {
    pub loc: String,
    /// `""` when absent.
    pub lastmod: String,
}

/// A parsed sitemap file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Sitemap {
    /// `<urlset>`: pages.
    Urls(Vec<Entry>),
    /// `<sitemapindex>`: other sitemaps.
    Index(Vec<Entry>),
}

/// Parse a `<urlset>` or a `<sitemapindex>`. `Err` for anything else or for broken XML.
pub(crate) fn parse(xml: &str) -> Result<Sitemap> {
    let mut reader = Reader::from_str(xml);
    let mut root: Option<bool> = None; // Some(true) for an index
    let mut entries: Vec<Entry> = Vec::new();
    let mut entry: Option<Entry> = None;
    let mut field = String::new();
    let mut text = String::new();
    // How deep the open element is: the root is 1, an entry (`<url>`, `<sitemap>`) 2, its own
    // `<loc>` and `<lastmod>` 3. Deeper ones (`<image:image><image:loc>` in WordPress sitemaps)
    // are other things' addresses and are ignored.
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| anyhow!("not a readable sitemap: {e}"))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                let name = local_name(e.name().as_ref());
                match (root, depth, name.as_str()) {
                    (None, _, "urlset") => root = Some(false),
                    (None, _, "sitemapindex") => root = Some(true),
                    (None, _, other) => bail!("not a sitemap (root element `{other}`)"),
                    (Some(_), 2, "url" | "sitemap") => entry = Some(Entry::default()),
                    (Some(_), 3, "loc" | "lastmod") if entry.is_some() => {
                        field = name;
                        text.clear();
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = local_name(e.name().as_ref());
                match (depth, name.as_str()) {
                    (2, "url" | "sitemap") => {
                        if let Some(done) = entry.take().filter(|d| !d.loc.is_empty()) {
                            if entries.len() < MAX_ENTRIES {
                                entries.push(done);
                            }
                        }
                    }
                    (3, "loc" | "lastmod") if name == field => {
                        if let Some(d) = entry.as_mut() {
                            let value = text.trim().to_string();
                            if name == "loc" {
                                d.loc = value;
                            } else {
                                d.lastmod = value;
                            }
                        }
                        field.clear();
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(t) if !field.is_empty() => {
                if let Ok(t) = t.decode() {
                    text.push_str(&t);
                }
            }
            Event::CData(c) if !field.is_empty() => {
                if let Ok(c) = c.decode() {
                    text.push_str(&c);
                }
            }
            Event::GeneralRef(r) if !field.is_empty() => {
                if let Ok(Some(ch)) = r.resolve_char_ref() {
                    text.push(ch);
                } else if let Ok(name) = r.decode() {
                    text.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            _ => {}
        }
    }
    match root {
        Some(true) => Ok(Sitemap::Index(entries)),
        Some(false) => Ok(Sitemap::Urls(entries)),
        None => bail!("not a sitemap"),
    }
}

/// An element name without its namespace prefix.
fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit(':').next().unwrap_or_default().to_string()
}

/// The sitemap address for the configured `site`: its scheme and host (and port), `/sitemap.xml`.
pub(crate) fn sitemap_url(site: &Url) -> Url {
    let mut url = site.clone();
    url.set_path("/sitemap.xml");
    url.set_query(None);
    url.set_fragment(None);
    url
}

/// The postings the sitemap of the configured `site` lists (see the module docs), newest
/// `<lastmod>` first, at most `max`. `listing` is the careers page itself, which is not a posting.
/// Never an error: a missing, walled or broken sitemap gives no candidates.
pub(crate) async fn read_sitemap(pages: &dyn Pages, site: &Url, listing: &Url, company: &str, max: usize) -> Vec<Candidate> {
    let mut entries = match read_file(pages, site, &sitemap_url(site)).await {
        Some(Sitemap::Urls(urls)) => urls,
        Some(Sitemap::Index(children)) => {
            let mut urls = Vec::new();
            for child in job_children(site, &children) {
                if let Some(Sitemap::Urls(found)) = read_file(pages, site, &child).await {
                    urls.extend(found);
                }
            }
            urls
        }
        None => return Vec::new(),
    };
    let total = entries.len();
    // Newest first; entries without a readable date keep their order, after the dated ones.
    entries.sort_by_key(|e| Reverse(lastmod(&e.lastmod)));
    let mut seen = std::collections::HashSet::new();
    let found: Vec<Candidate> = entries
        .into_iter()
        .filter_map(|e| {
            let mut url = Url::parse(&e.loc).ok()?;
            url.set_fragment(None);
            if !matches!(url.scheme(), "http" | "https") || !looks_like_posting(site, listing, &url) || !seen.insert(url.to_string()) {
                return None;
            }
            let title = title_from_url(&url)?;
            Some(Candidate { url: url.to_string(), title, company: company.to_string(), location: String::new(), snippet: String::new() })
        })
        .take(max)
        .collect();
    info!(entries = total, candidates = found.len(), "career page: sitemap read");
    found
}

/// Up to [`MAX_CHILDREN`] child sitemaps of an index: on the same site, named for jobs, not gzip.
fn job_children(site: &Url, children: &[Entry]) -> Vec<Url> {
    children
        .iter()
        .filter_map(|c| Url::parse(c.loc.trim()).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https") && same_site(site, u))
        .filter(|u| !u.path().to_ascii_lowercase().ends_with(".gz"))
        .filter(|u| {
            let path = u.path().to_ascii_lowercase();
            path.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| JOB_WORDS.contains(&w))
        })
        .take(MAX_CHILDREN)
        .collect()
}

/// One sitemap file read plain; `None` when it cannot be read, landed on another site, or is not
/// a sitemap.
async fn read_file(pages: &dyn Pages, site: &Url, url: &Url) -> Option<Sitemap> {
    let page = pages.plain(url.as_str()).await.ok()?;
    let landed = page_url(&page).ok()?;
    if !same_site(site, &landed) {
        return None;
    }
    parse(&page.html).ok()
}

fn lastmod(text: &str) -> Option<DateTime<Utc>> {
    parse_datetime(text).or_else(|| parse_date(text).and_then(|d| d.and_hms_opt(0, 0, 0)).map(|d| d.and_utc()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::source::web::fake::FakePages;

    const SITEMAP: &str = include_str!("../fixtures/sitemap.xml");
    const INDEX: &str = include_str!("../fixtures/sitemap_index.xml");

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn found(c: &[Candidate]) -> Vec<(&str, &str)> {
        c.iter().map(|c| (c.url.as_str(), c.title.as_str())).collect()
    }

    #[test]
    fn a_urlset_and_an_index_parse() {
        let Sitemap::Urls(urls) = parse(SITEMAP).unwrap() else { panic!("a urlset") };
        assert_eq!(urls.len(), 11);
        assert_eq!(urls[3], Entry { loc: "https://acme.test/careers/data-engineer".into(), lastmod: "2026-09-01".into() });
        assert!(urls.iter().any(|u| u.loc == "https://acme.test/careers/sales-lead?src=sitemap&lang=en"), "entities decoded");
        let Sitemap::Index(children) = parse(INDEX).unwrap() else { panic!("an index") };
        assert_eq!(children.len(), 8);
        assert!(matches!(parse(r#"<?xml version="1.0"?><x:urlset xmlns:x="s"><x:url><x:loc>https://a.test/jobs/1</x:loc></x:url></x:urlset>"#), Ok(Sitemap::Urls(u)) if u.len() == 1));
        // WordPress (Yoast) puts image addresses inside each entry: only the entry's own `<loc>` counts.
        let images = r#"<urlset xmlns:image="http://www.google.com/schemas/sitemap-image/1.1"><url><loc>https://a.test/jobs/1-rust-engineer</loc>
            <image:image><image:loc>https://a.test/wp-content/logo.png</image:loc></image:image><lastmod>2026-11-01</lastmod></url></urlset>"#;
        assert_eq!(parse(images).unwrap(), Sitemap::Urls(vec![Entry { loc: "https://a.test/jobs/1-rust-engineer".into(), lastmod: "2026-11-01".into() }]));
        for broken in ["", "<html><body>Not found</body></html>", "not xml at all"] {
            let ok_but_empty = matches!(parse(broken), Ok(Sitemap::Urls(ref u)) if u.is_empty());
            assert!(parse(broken).is_err() || ok_but_empty, "{broken}: {:?}", parse(broken));
        }
    }

    #[tokio::test]
    async fn a_sitemap_gives_same_site_postings_newest_first() {
        let site = url("https://acme.test/careers");
        let fake = FakePages::new().plain("https://acme.test/sitemap.xml", SITEMAP);
        let got = read_sitemap(&fake, &site, &site, "Acme", 500).await;
        assert_eq!(
            found(&got),
            [
                ("https://acme.test/careers/senior-rust-engineer", "senior rust engineer"),
                ("https://careers.acme.test/jobs/771234-devops-engineer", "devops engineer"),
                ("https://acme.test/careers/data-engineer", "data engineer"),
                ("https://acme.test/careers/sales-lead?src=sitemap&lang=en", "sales lead"),
                ("https://acme.test/careers/product-designer", "product designer"),
            ]
        );
        assert!(got.iter().all(|c| c.company == "Acme"));
        assert_eq!(fake.asked(), ["plain https://acme.test/sitemap.xml"]);
        assert_eq!(read_sitemap(&fake, &site, &site, "Acme", 2).await.len(), 2, "capped");
        // The address is the configured host's, whatever the careers page's path and query.
        assert_eq!(sitemap_url(&url("http://jobs.acme.test:8080/en/careers?x=1#y")).as_str(), "http://jobs.acme.test:8080/sitemap.xml");
    }

    #[tokio::test]
    async fn an_index_reads_at_most_three_job_sitemaps_on_the_site() {
        let site = url("https://acme.test/careers");
        let child = r#"<urlset><url><loc>https://acme.test/jobs/1234567-backend-engineer</loc></url></urlset>"#;
        let mut fake = FakePages::new().plain("https://acme.test/sitemap.xml", INDEX);
        for u in [
            "https://acme.test/sitemap-pages.xml",
            "https://acme.test/sitemap-jobs.xml.gz",
            "https://evil.test/sitemap-jobs.xml",
            "https://acme.test/sitemap-jobs-1.xml",
            "https://acme.test/sitemap-jobs-2.xml",
            "https://careers.acme.test/careers-sitemap.xml",
            "https://acme.test/sitemap-jobs-4.xml",
        ] {
            fake = fake.plain(u, child);
        }
        let got = read_sitemap(&fake, &site, &site, "Acme", 500).await;
        assert_eq!(found(&got), [("https://acme.test/jobs/1234567-backend-engineer", "backend engineer")], "the same posting once");
        assert_eq!(
            fake.asked(),
            [
                "plain https://acme.test/sitemap.xml",
                "plain https://acme.test/sitemap-jobs-1.xml",
                "plain https://acme.test/sitemap-jobs-2.xml",
                "plain https://careers.acme.test/careers-sitemap.xml",
            ]
        );
    }

    #[tokio::test]
    async fn a_missing_broken_or_moved_sitemap_gives_nothing() {
        let site = url("https://acme.test/careers");
        assert!(read_sitemap(&FakePages::new(), &site, &site, "Acme", 500).await.is_empty(), "404");
        let walled = FakePages::new().wall("https://acme.test/sitemap.xml", 403);
        assert!(read_sitemap(&walled, &site, &site, "Acme", 500).await.is_empty());
        let broken = FakePages::new().plain("https://acme.test/sitemap.xml", "<urlset><url><loc>https://acme.test/jobs/1-x</loc></url");
        assert!(read_sitemap(&broken, &site, &site, "Acme", 500).await.is_empty());
        let html = FakePages::new().plain("https://acme.test/sitemap.xml", "<!doctype html><html><body>Not here</body></html>");
        assert!(read_sitemap(&html, &site, &site, "Acme", 500).await.is_empty());
        let moved = FakePages::new().plain("https://acme.test/sitemap.xml", SITEMAP).moved("https://acme.test/sitemap.xml", "https://evil.test/sitemap.xml");
        assert!(read_sitemap(&moved, &site, &site, "Acme", 500).await.is_empty(), "a sitemap served by another site is not read");
    }
}
