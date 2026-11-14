//! Managing the job sources (Settings > Job sources): the rows of `sources` with purpose `jobs`.
//!
//! ```text
//! core.job_sources()?                                     // every job source, oldest first
//! core.add_job_source("career_page", "", "acme.test/careers", None)?   // checked by the adapter that will read it
//! core.set_job_source_enabled(id, false)?
//! core.set_job_source_session(id, Some(session_id))?      // the managed session a login-walled site is read in
//! core.remove_job_source(id)?                             // also skips its paused run
//! let hints = core.suggest_boards("acme.test/careers").await?;  // the job boards the page points to (read-only)
//! ```

use std::sync::OnceLock;

use anyhow::{bail, Context, Result};

use super::fetch::MAX_POSTING_BYTES;
use super::source::web::{normalize_page_url, read_plain_page, NetPages, Pages};
use super::source::{boards, check_source, BoardHint};
use super::{handoff, parse_source_thread};
use crate::engine::net::http::{Http, HttpLimits};
use crate::engine::store::{NewSource, Source, SourcePurpose};
use crate::engine::Core;

/// The kinds a job source can have, with the label the UI shows, in display order.
pub const JOB_SOURCE_KINDS: [(&str, &str); 10] = [
    ("greenhouse", "Greenhouse"),
    ("lever", "Lever"),
    ("ashby", "Ashby"),
    ("smartrecruiters", "SmartRecruiters"),
    ("recruitee", "Recruitee"),
    ("personio", "Personio"),
    ("workday", "Workday"),
    ("hn_hiring", "HN Who is hiring"),
    ("yc", "Y Combinator"),
    ("career_page", "Career page"),
];

/// What to type in the address field of a kind (a hint, not a rule).
pub fn source_url_hint(kind: &str) -> &'static str {
    match kind {
        "greenhouse" => "Board URL or name, e.g. boards.greenhouse.io/acme",
        "lever" => "jobs.lever.co/acme or acme",
        "ashby" => "jobs.ashbyhq.com/acme or acme",
        "smartrecruiters" => "jobs.smartrecruiters.com/Acme or Acme",
        "recruitee" => "acme.recruitee.com or acme",
        "personio" => "acme.jobs.personio.de (or .com) or acme",
        "workday" => "Board URL, e.g. https://acme.wd5.myworkdayjobs.com/External",
        "hn_hiring" => "No address needed",
        "yc" => "Optional: https://www.ycombinator.com/jobs",
        _ => "Careers page, e.g. https://acme.com/careers",
    }
}

impl Core {
    /// All job sources, oldest first.
    pub fn job_sources(&self) -> Result<Vec<Source>> {
        self.store.list_sources(SourcePurpose::Jobs)
    }

    /// Add a job source. The adapter of `kind` must accept `url` (a board name or address, a
    /// careers page, ...); an empty `name` becomes the board name, the host or the kind's title.
    /// `session_id` binds a managed browser session to it.
    pub fn add_job_source(&self, kind: &str, name: &str, url: &str, session_id: Option<i64>) -> Result<Source> {
        let (url, default_name) = check_source(kind.trim(), url)?;
        let name = match name.trim() {
            "" => default_name,
            given => given.to_string(),
        };
        if let Some(id) = session_id {
            self.store.get_session(id)?.with_context(|| format!("session {id} not found"))?;
        }
        let source = self.store.add_source(&NewSource { kind: kind.trim().to_string(), name, url, purpose: SourcePurpose::Jobs, session_id })?;
        tracing::info!(source_id = source.id, kind = %source.kind, "job source added");
        Ok(source)
    }

    pub fn set_job_source_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.job_source(id)?;
        self.store.set_source_enabled(id, enabled)
    }

    /// Bind (or with `None` unbind) the managed session this source is read in.
    pub fn set_job_source_session(&self, id: i64, session_id: Option<i64>) -> Result<()> {
        self.job_source(id)?;
        if let Some(session) = session_id {
            self.store.get_session(session)?.with_context(|| format!("session {session} not found"))?;
        }
        self.store.set_source_session(id, session_id)
    }

    /// Remove a job source. A run of it that waits for the user is skipped (and its browser window
    /// closed); the leads it found stay.
    pub fn remove_job_source(&self, id: i64) -> Result<()> {
        self.job_source(id)?;
        for waiting in self.discover_waiting()?.into_iter().filter(|w| w.source_id == id) {
            handoff::discard_window(&self.paths, &waiting.thread);
            self.graphs.cancel(&waiting.thread)?;
        }
        self.store.delete_source(id)?;
        tracing::info!(source_id = id, "job source removed");
        Ok(())
    }

    /// The job boards a careers page points to (a Greenhouse iframe, Lever links, ...), each one a
    /// board address that [`check_source`] accepts: what the add form offers to add instead. One
    /// plain read of the page through the safe HTTP client, no browser; nothing is stored and no
    /// board is read. A page behind a wall has no hints. `url` must pass the career-page check.
    pub async fn suggest_boards(&self, url: &str) -> Result<Vec<BoardHint>> {
        suggest_boards_with(&NetPages::new(suggest_client()?, None), url).await
    }

    fn job_source(&self, id: i64) -> Result<Source> {
        match self.store.get_source(id)? {
            Some(source) if source.purpose == SourcePurpose::Jobs => Ok(source),
            Some(_) => bail!("source {id} is not a job source"),
            None => bail!("source {id} no longer exists"),
        }
    }

    /// The thread of a source paused for the user, if any (for "Open browser" on its row).
    pub fn waiting_thread_of(&self, source_id: i64) -> Result<Option<String>> {
        Ok(self.graphs.waiting()?.into_iter().find(|w| parse_source_thread(&w.thread).is_some_and(|(_, id)| id == source_id)).map(|w| w.thread))
    }
}

/// The one client of [`Core::suggest_boards`], for the life of the process, so its per-host delay
/// holds across calls (the add form may ask about several addresses of one site in a row). The app
/// runs every engine call on the one engine runtime, which the client's pooled connections need.
fn suggest_client() -> Result<Http> {
    static CLIENT: OnceLock<Http> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = Http::new(&HttpLimits { max_bytes: MAX_POSTING_BYTES, ..HttpLimits::default() })?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

/// [`Core::suggest_boards`] over any page reader.
pub(crate) async fn suggest_boards_with(pages: &dyn Pages, url: &str) -> Result<Vec<BoardHint>> {
    check_source("career_page", url)?;
    let page_url = normalize_page_url(url).context("career page source")?;
    let Some(page) = read_plain_page(pages, page_url.as_str()).await? else { return Ok(Vec::new()) };
    let hints = boards::page_hints(&page);
    tracing::info!(boards = hints.len(), "careers page checked for job boards");
    Ok(hints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    #[test]
    fn sources_are_added_named_toggled_bound_and_removed() {
        let (_dir, core) = core();
        let gh = core.add_job_source("greenhouse", "", "https://boards.greenhouse.io/acme", None).unwrap();
        assert_eq!((gh.name.as_str(), gh.enabled, gh.session_id), ("acme", true, None));
        let yc = core.add_job_source("yc", "", "", None).unwrap();
        assert_eq!((yc.name.as_str(), yc.url.as_str()), ("Y Combinator jobs", "https://www.ycombinator.com/jobs"));
        let page = core.add_job_source("career_page", "Acme careers", "acme.test/careers", None).unwrap();
        assert_eq!(page.name, "Acme careers");
        // A trends source is not listed here and cannot be changed through these calls.
        let trend = core.store.add_source(&NewSource { kind: "rss".into(), purpose: SourcePurpose::Trends, ..Default::default() }).unwrap();
        assert_eq!(core.job_sources().unwrap().iter().map(|s| s.id).collect::<Vec<_>>(), [gh.id, yc.id, page.id]);
        assert!(core.set_job_source_enabled(trend.id, false).unwrap_err().to_string().contains("not a job source"));
        assert!(core.remove_job_source(trend.id).is_err());

        core.set_job_source_enabled(gh.id, false).unwrap();
        assert!(!core.store.get_source(gh.id).unwrap().unwrap().enabled);

        let session = core.store.add_session(&crate::engine::store::NewSession { name: "Acme".into(), start_url: "https://acme.test/login".into() }).unwrap();
        core.set_job_source_session(page.id, Some(session.id)).unwrap();
        assert_eq!(core.store.get_source(page.id).unwrap().unwrap().session_id, Some(session.id));
        assert!(core.set_job_source_session(page.id, Some(session.id + 99)).unwrap_err().to_string().contains("not found"));
        core.set_job_source_session(page.id, None).unwrap();
        assert_eq!(core.store.get_source(page.id).unwrap().unwrap().session_id, None);

        core.remove_job_source(yc.id).unwrap();
        assert!(core.store.get_source(yc.id).unwrap().is_none());
        assert!(core.remove_job_source(yc.id).unwrap_err().to_string().contains("no longer exists"));
    }

    #[test]
    fn a_source_the_adapter_cannot_read_is_refused_with_the_reason() {
        let (_dir, core) = core();
        for (kind, url, why) in [
            ("greenhouse", "", "no company board"),
            ("lever", "https://evil.test/acme", "evil.test"),
            ("career_page", "", "no page address"),
            ("career_page", "javascript:alert(1)", "not a valid address"),
            ("career_page", "ftp://x.test/careers", "must be http"),
            ("career_page", "https://boards.greenhouse.io/acme", "job board shared by many companies"),
            ("career_page", "https://github.io/careers", "hosting platform"),
            ("yc", "https://evil.test/jobs", "not a ycombinator.com"),
            ("linkedin", "https://www.linkedin.com/jobs", "no adapter"),
        ] {
            let err = format!("{:#}", core.add_job_source(kind, "x", url, None).unwrap_err());
            assert!(err.contains(why), "{kind} {url}: {err}");
        }
        assert!(core.job_sources().unwrap().is_empty(), "nothing was stored");
        let session_err = core.add_job_source("hn_hiring", "", "", Some(42)).unwrap_err();
        assert!(session_err.to_string().contains("not found"));
        assert!(core.job_sources().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_careers_page_suggests_the_boards_it_points_to_without_reading_them() {
        use crate::engine::discover::source::web::fake::FakePages;
        let html = include_str!("fixtures/career_page_embed.html");
        let pages = FakePages::new().plain("https://acme.test/careers", html).wall("https://walled.test/careers", 403);
        let hints = suggest_boards_with(&pages, "acme.test/careers").await.unwrap();
        let shown: Vec<_> = hints.iter().map(|h| (h.kind, h.input.as_str(), h.label())).collect();
        assert_eq!(shown, [("greenhouse", "boards.greenhouse.io/acme", "Greenhouse"), ("lever", "jobs.lever.co/acme", "Lever")]);
        assert_eq!(pages.asked(), ["plain https://acme.test/careers"], "one plain read, no board, no browser");
        // A wall: no hints, no error.
        assert!(suggest_boards_with(&pages, "https://walled.test/careers").await.unwrap().is_empty());
        // The address is checked as a careers page first, before anything is read.
        for bad in ["", "javascript:alert(1)", "https://boards.greenhouse.io/acme"] {
            assert!(suggest_boards_with(&pages, bad).await.is_err(), "{bad}");
        }
        assert_eq!(pages.asked().len(), 2);
        // Another failure is the error.
        assert!(suggest_boards_with(&pages, "https://gone.test/careers").await.is_err());
    }

    #[tokio::test]
    async fn the_live_suggestion_refuses_a_private_address_without_a_read() {
        // A real page on loopback, which would answer if it were asked.
        let fixture = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        fixture.set_nonblocking(true).unwrap();
        let port = fixture.local_addr().unwrap().port();
        let (_dir, core) = core();
        let url = format!("http://127.0.0.1:{port}/careers");
        let err = core.suggest_boards(&url).await.unwrap_err();
        let chain: Vec<String> = err.chain().map(ToString::to_string).collect();
        assert!(chain.iter().any(|e| e.contains("private network")), "{chain:?}");
        // A local name is refused earlier, as no company site.
        assert!(format!("{:#}", core.suggest_boards(&format!("http://localhost:{port}/careers")).await.unwrap_err()).contains("not a company site"));
        match fixture.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("the fixture was asked: {other:?}"),
        }
    }

    #[test]
    fn every_listed_kind_is_accepted_by_the_checker() {
        for (kind, _) in JOB_SOURCE_KINDS {
            let url = match kind {
                "greenhouse" | "lever" | "ashby" | "smartrecruiters" | "recruitee" | "personio" => "acme",
                "career_page" => "https://acme.test/careers",
                "workday" => "https://acme.wd5.myworkdayjobs.com/External",
                _ => "",
            };
            assert!(check_source(kind, url).is_ok(), "{kind}");
            assert!(!source_url_hint(kind).is_empty());
        }
    }
}
