//! Checking a job source before it is stored: the address must pass the adapter's own check
//! ([`check_source`]), and the source is read once, so a page that lists no jobs is refused at Add
//! instead of showing up later as an empty run.
//!
//! ```text
//! let checked = core.check_job_source("career_page", "acme.com/careers", None).await?;  // { kind, url, name, jobs, titles }
//! let source = core.add_job_source_checked("career_page", "", "acme.com/careers", None).await?;
//! ```
//!
//! | Kind | Read | Refused when |
//! |------|------|--------------|
//! | a company board (`greenhouse`, `lever`, ...) | its listing, one page, no role filter | the board does not exist (HTTP 404 / 410); a board with no open job is accepted |
//! | `hn_hiring`, `yc` | none | never (the address check only) |
//! | `career_page` (a job page) | the page as a run reads it: plain, its sitemap, then the user's browser (with the bound session) | no posting found: the message names the board the page points to, or the login / captcha it showed |
//!
//! A job page whose address is a company board is checked (and stored) as that board. Code reads
//! every address; nothing on a page chooses one.

use std::sync::{Arc, OnceLock};

use anyhow::{bail, Result};
use tracing::info;

use super::fetch::MAX_POSTING_BYTES;
use super::source::{adapter_for, check_source, page_company, CareerPage, Http as SourceHttp, NetHttp, NetPages, Pages, SearchCtx};
use super::sources::kind_label;
use crate::browser::challenge::ChallengeFound;
use crate::browser::render::Renderer;
use crate::net::http::{Http, HttpLimits};
use crate::prefs::Preferences;
use crate::store::{Source, SourcePurpose};
use crate::Core;

/// Most postings asked of a board in one check (a board that pages its API answers fewer).
const CHECK_PAGE_SIZE: usize = 500;

/// Largest JSON answer read in a check (a Lever listing carries every description).
const JSON_MAX_BYTES: usize = 32 * 1024 * 1024;

/// A job source that passed [`Core::check_job_source`]: what to store, and what the read found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckedJobSource {
    /// The kind to store (a board's when a job page's address is a company board).
    pub kind: String,
    /// The address to store.
    pub url: String,
    /// A name for it when the user gave none.
    pub name: String,
    /// The postings the read found: a job page's every posting, a board's first listing page
    /// (all of it for most boards); 0 for the kinds that are not read (`hn_hiring`, `yc`).
    pub jobs: usize,
    /// Their titles (as many as `jobs`), to tell how many fit the wanted roles.
    pub titles: Vec<String>,
}

impl Core {
    /// Check a job source the way it will be read, without storing anything (see the module
    /// docs): `Err` with a message the user can act on when its address is refused or a job page
    /// lists no jobs. A page that needs the browser while the extension is not connected fails with
    /// the browser's own error, which `health::classify` turns into its problem. Cheap to call
    /// again: the HTTP client (and its per-host delays) is shared by every check.
    pub async fn check_job_source(&self, kind: &str, url: &str, session_id: Option<i64>) -> Result<CheckedJobSource> {
        let client = check_client()?;
        let http: Arc<dyn SourceHttp> = Arc::new(NetHttp::new(client.clone()));
        let pages: Arc<dyn Pages> = Arc::new(NetPages::new(client.with_max_bytes(MAX_POSTING_BYTES), Some(Renderer::new(self, false))));
        check_job_source_with(http, pages, kind, url, session_id).await
    }

    /// The add form's Add: [`check_job_source`](Self::check_job_source), then store the source with
    /// the kind and address the check resolved ([`Core::add_job_source`]).
    pub async fn add_job_source_checked(&self, kind: &str, name: &str, url: &str, session_id: Option<i64>) -> Result<Source> {
        self.require_session(session_id)?;
        let checked = self.check_job_source(kind, url, session_id).await?;
        let source = self.add_job_source(&checked.kind, name, &checked.url, session_id)?;
        info!(source_id = source.id, kind = %source.kind, jobs = checked.jobs, "job source checked and added");
        Ok(source)
    }
}

/// The one client of every check, for the life of the process, so its per-host delay holds across
/// checks (a list of suggestions is checked one after the other).
fn check_client() -> Result<Http> {
    static CLIENT: OnceLock<Http> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = Http::new(&HttpLimits { max_bytes: JSON_MAX_BYTES, ..HttpLimits::default() })?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

/// [`Core::check_job_source`] over any API client and page reader.
pub(crate) async fn check_job_source_with(
    http: Arc<dyn SourceHttp>,
    pages: Arc<dyn Pages>,
    kind: &str,
    url: &str,
    session_id: Option<i64>,
) -> Result<CheckedJobSource> {
    let checked = check_source(kind.trim(), url)?;
    let source = Source {
        id: 0,
        kind: checked.kind.clone(),
        name: checked.name.clone(),
        url: checked.url.clone(),
        purpose: SourcePurpose::Jobs,
        enabled: true,
        session_id,
        last_run_at: None,
        last_status: None,
    };
    let titles = match checked.kind.as_str() {
        "hn_hiring" | "yc" => Vec::new(),
        "career_page" => read_page(pages, &source).await?,
        board => read_board(http, &source, board).await?,
    };
    info!(kind = %checked.kind, jobs = titles.len(), "job source checked");
    Ok(CheckedJobSource { kind: checked.kind, url: checked.url, name: checked.name, jobs: titles.len(), titles })
}

/// The titles of a board's first listing page. A board that does not exist is refused; a board
/// with no open job is not (it exists, and will list jobs later).
async fn read_board(http: Arc<dyn SourceHttp>, source: &Source, kind: &str) -> Result<Vec<String>> {
    let label = kind_label(kind);
    let adapter = adapter_for(source, http)?;
    match adapter.search(&Preferences::default(), &SearchCtx { cursor: None, page_size: CHECK_PAGE_SIZE }).await {
        Ok(page) => Ok(page.candidates.into_iter().map(|c| c.title).collect()),
        Err(e) if is_missing(&e) => bail!("no such board on {label}: check the address"),
        // A network failure keeps its text, so it is shown with Retry.
        Err(e) => Err(e.context(format!("checking the {label} board"))),
    }
}

/// The answer says the board is not there.
fn is_missing(e: &anyhow::Error) -> bool {
    let text = format!("{e:#}");
    text.contains("HTTP 404") || text.contains("HTTP 410")
}

/// The titles of a job page's postings, read like a run reads them (plain, sitemap, browser).
/// No posting is a refusal naming what to do instead.
async fn read_page(pages: Arc<dyn Pages>, source: &Source) -> Result<Vec<String>> {
    let page = CareerPage::new(pages, source, &page_company(source))?;
    let (read, hints) = page.read_for_check().await;
    if let Ok(found) = &read {
        if !found.is_empty() {
            return Ok(found.iter().map(|c| c.title.clone()).collect());
        }
    }
    // A page that embeds a board names it even when the browser could not render the page.
    if let Some(board) = hints.first() {
        let label = board.label();
        bail!("This page lists its jobs on {label}: add the {label} board ({})", board.input);
    }
    match read {
        Err(e) if e.chain().any(|c| c.is::<ChallengeFound>()) => {
            bail!("the page showed a login or captcha: sign in to the site (Settings > Browser) and add it again")
        }
        // The browser's own error (not connected, not allowed in Incognito) or a network failure.
        Err(e) => Err(e.context("reading the job page")),
        Ok(_) => bail!("no job postings found on this page"),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use async_trait::async_trait;

    use super::*;
    use crate::browser::bridge::NOT_CONNECTED;
    use crate::discover::source::web::fake::{pages, FakePages};
    use crate::discover::source::web::Page;
    use crate::discover::source::FakeHttp;
    use crate::graph::nodes::ChallengeKind;
    use crate::health::{classify, Problem};

    const LIST: &str = "https://acme.test/careers";
    const SITEMAP_URL: &str = "https://acme.test/sitemap.xml";
    const PAGE: &str = include_str!("fixtures/career_page.html");
    const EMBED: &str = include_str!("fixtures/career_page_embed.html");
    const GH_JOBS: &str = "https://boards-api.greenhouse.io/v1/boards/acme/jobs";

    fn no_http() -> Arc<FakeHttp> {
        FakeHttp::with([])
    }

    async fn check(http: Arc<FakeHttp>, pages: Arc<FakePages>, kind: &str, url: &str) -> Result<CheckedJobSource> {
        check_job_source_with(http, pages, kind, url, None).await
    }

    async fn refusal(pages: Arc<FakePages>, url: &str) -> String {
        format!("{:#}", check(no_http(), pages, "career_page", url).await.unwrap_err())
    }

    #[tokio::test]
    async fn a_page_with_postings_is_accepted_with_its_count() {
        let fake = pages(FakePages::new().plain(LIST, PAGE));
        let checked = check(no_http(), fake.clone(), "career_page", "acme.test/careers").await.unwrap();
        assert_eq!((checked.kind.as_str(), checked.url.as_str(), checked.name.as_str(), checked.jobs), ("career_page", "acme.test/careers", "acme.test", 4));
        assert_eq!(checked.titles.len(), 4);
        assert!(checked.titles.contains(&"Data Engineer".to_string()), "{:?}", checked.titles);
        assert_eq!(fake.asked(), [format!("plain {LIST}")], "no role filter, one read");
    }

    #[tokio::test]
    async fn a_page_that_only_points_to_a_board_is_refused_naming_the_board() {
        let fake = pages(FakePages::new().plain(LIST, EMBED).plain(SITEMAP_URL, "<urlset></urlset>").rendered(LIST, EMBED));
        let err = refusal(fake, LIST).await;
        assert!(err.contains("lists its jobs on Greenhouse: add the Greenhouse board (boards.greenhouse.io/acme)"), "{err}");
        // Also when the browser could not render it.
        let err = refusal(pages(FakePages::new().plain(LIST, EMBED)), LIST).await;
        assert!(err.contains("add the Greenhouse board"), "{err}");
    }

    #[tokio::test]
    async fn a_login_or_captcha_is_refused_with_what_to_do() {
        let fake = pages(FakePages::new().wall(LIST, 403).challenge(LIST, ChallengeKind::Captcha));
        let err = refusal(fake, LIST).await;
        assert!(err.contains("login or captcha") && err.contains("Settings > Browser"), "{err}");
        assert_eq!(classify(&err), Problem::Other, "shown under the form");
    }

    #[tokio::test]
    async fn a_page_without_postings_is_refused() {
        let empty = "<html><body><p>About us</p><a href=\"/about\">About</a></body></html>";
        let fake = pages(FakePages::new().plain(LIST, empty).plain(SITEMAP_URL, "<urlset></urlset>").rendered(LIST, empty));
        assert!(refusal(fake, LIST).await.contains("no job postings found on this page"));
        // A page of a shared host is read like any other: postings only on its exact host.
        let list = "https://www.linkedin.com/jobs/search?keywords=rust";
        let html = r#"<a href="/jobs/view/rust-engineer-4011001">Rust Engineer</a><a href="https://evil.linkedin.com/jobs/view/1234567">Elsewhere</a>"#;
        let checked = check(no_http(), pages(FakePages::new().plain(list, html)), "career_page", list).await.unwrap();
        assert_eq!((checked.jobs, checked.name.as_str()), (1, "linkedin.com"));
    }

    /// A browser that is not connected.
    struct Disconnected;

    #[async_trait]
    impl Pages for Disconnected {
        async fn plain(&self, url: &str) -> Result<Page> {
            Ok(Page { url: url.to_string(), html: "<div id=root></div>".into() })
        }
        async fn rendered(&self, _source: &Source, _url: &str) -> Result<Page> {
            Err(anyhow!("rendering the page: {NOT_CONNECTED}"))
        }
    }

    #[tokio::test]
    async fn a_page_that_needs_the_browser_while_it_is_not_connected_is_the_browser_problem() {
        let err = check_job_source_with(no_http(), Arc::new(Disconnected), "career_page", LIST, None).await.unwrap_err();
        assert_eq!(classify(&format!("{err:#}")), Problem::NoBrowser { incognito: false }, "{err:#}");
    }

    #[tokio::test]
    async fn a_board_must_exist_and_may_be_empty() {
        let missing = format!("{:#}", check(no_http(), pages(FakePages::new()), "greenhouse", "acme").await.unwrap_err());
        assert!(missing.contains("no such board on Greenhouse"), "{missing}");

        let empty = FakeHttp::with([(GH_JOBS, r#"{"jobs": []}"#)]);
        let checked = check(empty, pages(FakePages::new()), "greenhouse", "boards.greenhouse.io/acme").await.unwrap();
        assert_eq!((checked.kind.as_str(), checked.name.as_str(), checked.jobs), ("greenhouse", "acme", 0));

        let busy = FakeHttp::with_statuses([], [(GH_JOBS, 503)]);
        let err = format!("{:#}", check(busy, pages(FakePages::new()), "greenhouse", "acme").await.unwrap_err());
        assert_eq!(classify(&err), Problem::Network, "a network failure keeps Retry: {err}");
    }

    #[tokio::test]
    async fn a_board_pasted_as_a_job_page_is_read_as_that_board() {
        let http = FakeHttp::with([(GH_JOBS, include_str!("fixtures/greenhouse_jobs.json"))]);
        let fake = pages(FakePages::new());
        let checked = check(http.clone(), fake.clone(), "career_page", "https://boards.greenhouse.io/acme").await.unwrap();
        assert_eq!((checked.kind.as_str(), checked.url.as_str(), checked.name.as_str()), ("greenhouse", "boards.greenhouse.io/acme", "acme"));
        assert!(checked.jobs > 0 && checked.jobs == checked.titles.len());
        assert_eq!(http.requested(), [GH_JOBS]);
        assert!(fake.asked().is_empty(), "the page itself is not read");
    }

    #[tokio::test]
    async fn the_feeds_are_not_read() {
        let (http, fake) = (no_http(), pages(FakePages::new()));
        for kind in ["hn_hiring", "yc"] {
            assert_eq!(check(http.clone(), fake.clone(), kind, "").await.unwrap().jobs, 0, "{kind}");
        }
        assert!(http.requested().is_empty() && fake.asked().is_empty());
        assert!(check(http, fake, "nope", "x").await.is_err());
    }
}
