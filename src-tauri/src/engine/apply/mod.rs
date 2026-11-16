//! Auto-apply: review -> confirm -> submit.
//!
//! ```text
//! // Apply tab, idle: what would happen?
//! let info = core.apply_info(app_id)?;                // ATS, apply URL, session, dry-run
//!
//! // "Review & Submit": open the form in a tab of the user's Chrome, read + answer + fill it, keep
//! // the tab for the review.
//! let rx = runtime::spawn(async move { apply::prepare(&core, id, ApplyOptions { dry_run }, |stage| ..).await });
//! // -> Prepared::Review(review) | Prepared::Handoff(handoff)
//!
//! // More steps (LinkedIn): fill the highlighted rows, then
//! apply::advance(&core, id, edits, |stage| ..).await?;
//!
//! // The explicit "Submit" click (never called by anything else):
//! match apply::submit(&core, id, edits).await? {
//!     Submitted::Applied { proof } => ..,      // status Applied, proof screenshot in shots/
//!     Submitted::DryRun => ..,                 // nothing was sent
//!     Submitted::Handoff(h) => ..,             // status Failed; apply::open_visible(..) (the tab to the front) then finish_manually(..)
//! }
//! ```
//!
//! The browser tab of a prepared application stays open in a process-wide registry ([`flow`]) between
//! the review and the submit click; [`live`] tells the UI what to show for an application (also after
//! switching tabs). Call [`close_all`] when the app quits.
//!
//! Modules: [`ats`] detection + per-ATS adapters, [`js`] injected scripts, [`form`] page driving,
//! [`answer`] deterministic prefill + LLM answers + validation, [`detect`] CAPTCHA / success heuristics,
//! [`flow`] the state machine. Answers the user asks to remember are stored (`Store::save_answer`).

pub mod answer;
pub mod ats;
pub mod detect;
pub mod fields;
pub mod flow;
pub mod form;
pub mod js;

#[cfg(test)]
mod integration;

use anyhow::{bail, Context, Result};

pub use answer::Facts;
pub use ats::{detect_ats, Ats};
pub use fields::{Answer, FieldKind, FormField, Source};
pub use flow::{
    advance, close_all, discard, finish_manually, latest_proof, live, open_visible, prepare, submit, Edit, Handoff,
    Live, Prepared, Review, Row, Stage, Submitted, while_no_run,
};

use crate::engine::browser::SessionStatus;
use crate::engine::pipeline::analyze::{valid_http_url, Analysis};
use crate::engine::store::{Application, Session};
use crate::engine::Core;

/// Env var: set to `1` to force every run to be a dry run (fill, never submit).
pub const DRY_RUN_ENV: &str = "JOBPILOT_DRY_RUN";

/// `JOBPILOT_DRY_RUN` is set to something other than empty / `0` / `false`.
pub fn dry_run_forced() -> bool {
    std::env::var(DRY_RUN_ENV).is_ok_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ApplyOptions {
    /// Fill the form but never submit it (the status does not change either).
    pub dry_run: bool,
}

/// Where the apply URL came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlSource {
    /// Set on the application (by the user).
    Application,
    /// Found in the job description by the analysis step.
    Analysis,
}

/// The browser session a run will use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// No login needed: a private (Incognito) tab is enough.
    NotNeeded,
    Matched { id: i64, name: String, status: SessionStatus },
    /// The ATS needs a login and no session covers this domain.
    Missing { domain: String },
}

/// Everything the idle Apply tab shows.
#[derive(Clone, Debug)]
pub struct ApplyInfo {
    pub ats: Ats,
    pub apply_url: Option<String>,
    pub url_source: Option<UrlSource>,
    pub session: SessionState,
    pub dry_run_forced: bool,
    /// The JobPilot extension is connected: the form can be opened in a tab of the user's Chrome.
    pub browser_connected: bool,
    /// The extension may open private (Incognito) tabs, where forms that need no login open.
    pub private_tabs: bool,
}

impl ApplyInfo {
    /// The Settings section that fixes the [`blocker`](Self::blocker), if one does.
    pub fn blocker_settings(&self) -> Option<&'static str> {
        if self.apply_url.is_none() {
            None
        } else if matches!(self.session, SessionState::Missing { .. }) || !self.browser_connected {
            Some("Browser")
        } else {
            None
        }
    }

    /// Why a run cannot start (shown in the idle state), or `None`.
    pub fn blocker(&self) -> Option<String> {
        if self.apply_url.is_none() {
            return Some("No apply URL yet: enter the link of the application form.".into());
        }
        if let SessionState::Missing { domain } = &self.session {
            return Some(format!(
                "{} needs you signed in to {domain} in your browser: add the site in Settings > Browser.",
                self.ats.label()
            ));
        }
        if !self.browser_connected {
            return Some(
                "Connect the JobPilot extension: open Chrome with the extension and pair it in Settings > Browser."
                    .into(),
            );
        }
        if self.session == SessionState::NotNeeded && !self.private_tabs {
            return Some(
                "Allow the JobPilot extension in Incognito (chrome://extensions > JobPilot > Details > Allow in Incognito): forms that need no login open in a private tab."
                    .into(),
            );
        }
        None
    }
}

/// The apply URL of an application: its own, else the one the JD analysis found.
pub fn resolve_apply_url(app: &Application) -> Option<(String, UrlSource)> {
    if let Some(url) = app.apply_url.as_deref().and_then(valid_http_url) {
        return Some((url, UrlSource::Application));
    }
    let analysis: Analysis = serde_json::from_str(app.analysis_json.as_deref()?).ok()?;
    valid_http_url(&analysis.apply_url).map(|u| (u, UrlSource::Analysis))
}

/// Pick the session for an ATS that needs a login: one whose domains (or start page) cover the host
/// of `url`. A session whose last check passed wins over an expired one.
pub fn pick_session(ats: Ats, url: &str, sessions: &[Session]) -> SessionState {
    let Some(domain) = ats.session_domain() else { return SessionState::NotNeeded };
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    let covers = |d: &str| {
        let d = d.trim_start_matches('.').to_ascii_lowercase();
        !d.is_empty() && (host == d || host.ends_with(&format!(".{d}")) || d.ends_with(&format!(".{host}")))
    };
    let mut matching: Vec<&Session> = sessions
        .iter()
        .filter(|s| {
            s.domains().iter().any(|d| covers(d))
                || url::Url::parse(&s.start_url).ok().and_then(|u| u.host_str().map(covers)).unwrap_or(false)
        })
        .collect();
    matching.sort_by_key(|s| match s.session_status() {
        SessionStatus::Ok => 0,
        SessionStatus::New => 1,
        SessionStatus::Expired => 2,
    });
    match matching.first() {
        Some(s) => SessionState::Matched { id: s.id, name: s.name.clone(), status: s.session_status() },
        None => SessionState::Missing { domain: domain.to_string() },
    }
}

/// What the Apply tab shows before anything runs.
pub fn apply_info(core: &Core, app: &Application) -> ApplyInfo {
    let (apply_url, url_source) = match resolve_apply_url(app) {
        Some((url, source)) => (Some(url), Some(source)),
        None => (None, None),
    };
    let url = apply_url.as_deref().unwrap_or("");
    let ats = detect_ats(url);
    let sessions = core.store.list_sessions().unwrap_or_default();
    let status = core.browser().status();
    ApplyInfo {
        ats,
        session: pick_session(ats, url, &sessions),
        apply_url,
        url_source,
        dry_run_forced: dry_run_forced(),
        browser_connected: status.connected,
        private_tabs: status.incognito_allowed,
    }
}

impl Core {
    /// [`apply_info`] by application id.
    pub fn apply_info(&self, app_id: i64) -> Result<ApplyInfo> {
        let app = self.store.get_application(app_id)?.with_context(|| format!("application {app_id} not found"))?;
        Ok(apply_info(self, &app))
    }

    /// Save the apply URL the user typed (http/https only). Returns the cleaned URL.
    pub fn set_apply_url(&self, app_id: i64, url: &str) -> Result<String> {
        let candidate = url.trim();
        let candidate = if candidate.contains("://") { candidate.to_string() } else { format!("https://{candidate}") };
        let Some(clean) = valid_http_url(&candidate) else { bail!("`{}` is not a web address (http or https)", url.trim()) };
        let mut app = self.store.get_application(app_id)?.with_context(|| format!("application {app_id} not found"))?;
        app.apply_url = Some(clean.clone());
        self.store.save_application(&app)?;
        Ok(clean)
    }

    /// The application answers the user asked to remember, oldest first.
    pub fn saved_answers(&self) -> Result<Vec<(String, String)>> {
        self.store.saved_answers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::store::NewApplication;
    use crate::engine::{Paths, Secrets};
    use chrono::Utc;

    fn session(id: i64, name: &str, start: &str, domains: &[&str], status: &str) -> Session {
        Session {
            id,
            name: name.into(),
            start_url: start.into(),
            domains_json: serde_json::to_string(domains).unwrap(),
            status: status.into(),
            last_verified_at: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn sessions_are_matched_by_domain() {
        let sessions = vec![
            session(1, "Google", "https://accounts.google.com/", &["google.com"], "ok"),
            session(2, "LinkedIn old", "https://www.linkedin.com/login", &["linkedin.com"], "expired"),
            session(3, "LinkedIn", "https://www.linkedin.com/login", &["www.linkedin.com", ".linkedin.com"], "ok"),
        ];
        let url = "https://www.linkedin.com/jobs/view/1/";
        assert_eq!(
            pick_session(Ats::LinkedIn, url, &sessions),
            SessionState::Matched { id: 3, name: "LinkedIn".into(), status: SessionStatus::Ok },
            "the healthy session wins"
        );
        assert_eq!(
            pick_session(Ats::LinkedIn, url, &sessions[..1]),
            SessionState::Missing { domain: "linkedin.com".into() }
        );
        assert_eq!(pick_session(Ats::Greenhouse, "https://boards.greenhouse.io/a/jobs/1", &sessions), SessionState::NotNeeded);

        // A session known only by its start page still matches Workday tenants.
        let wd = vec![session(9, "Acme careers", "https://acme.wd5.myworkdayjobs.com/en-US/External", &[], "new")];
        assert!(matches!(
            pick_session(Ats::Workday, "https://acme.wd5.myworkdayjobs.com/en-US/External/job/X", &wd),
            SessionState::Matched { id: 9, .. }
        ));
    }

    #[test]
    fn apply_url_comes_from_the_application_then_the_analysis() {
        let mut app = Application {
            id: 1,
            slug: "a".into(),
            company: "Acme".into(),
            role: "Dev".into(),
            source: None,
            apply_url: None,
            status: crate::engine::store::Status::CvReady,
            dir: "/tmp/a".into(),
            analysis_json: None,
            gaps_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            insight_json: None,
            interview_round: None,
            dismissed_from: None,
            dismiss_reason: None,
            dismissed_at: None,
        };
        assert_eq!(resolve_apply_url(&app), None);
        app.analysis_json = Some(r#"{"apply_url": "https://jobs.lever.co/acme/1"}"#.into());
        assert_eq!(resolve_apply_url(&app), Some(("https://jobs.lever.co/acme/1".into(), UrlSource::Analysis)));
        app.analysis_json = Some(r#"{"apply_url": "javascript:alert(1)"}"#.into());
        assert_eq!(resolve_apply_url(&app), None, "non-http URLs from the analysis are ignored");
        app.apply_url = Some("https://boards.greenhouse.io/acme/jobs/2".into());
        assert_eq!(resolve_apply_url(&app), Some(("https://boards.greenhouse.io/acme/jobs/2".into(), UrlSource::Application)));
    }

    #[test]
    fn apply_url_can_be_set_and_is_validated() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        let app = core
            .store
            .create_application(&NewApplication { slug: "x".into(), company: "X".into(), role: "Y".into(), dir: dir.path().join("x"), ..Default::default() })
            .unwrap();
        assert_eq!(core.set_apply_url(app.id, " jobs.lever.co/x/1 ").unwrap(), "https://jobs.lever.co/x/1");
        assert!(core.set_apply_url(app.id, "javascript:alert(1)").is_err());
        assert!(core.set_apply_url(app.id, "file:///etc/passwd").is_err());
        let info = core.apply_info(app.id).unwrap();
        assert_eq!(info.ats, Ats::Lever);
        assert_eq!(info.url_source, Some(UrlSource::Application));
        assert_eq!(info.session, SessionState::NotNeeded);
        // No extension is connected to this data dir's bridge: the form cannot be opened yet.
        assert!(!info.browser_connected);
        assert!(info.blocker().is_some_and(|b| b.contains("Connect the JobPilot extension")), "{:?}", info.blocker());
        assert_eq!(info.blocker_settings(), Some("Browser"));

        // Connected, but not allowed in Incognito: a form that needs no login cannot open its private tab.
        let connected = ApplyInfo { browser_connected: true, ..info.clone() };
        assert!(connected.blocker().is_some_and(|b| b.contains("Allow in Incognito")), "{:?}", connected.blocker());
        assert_eq!(connected.blocker_settings(), None, "the fix is in Chrome");
        assert_eq!(ApplyInfo { private_tabs: true, ..connected.clone() }.blocker(), None);
        // A form behind a login opens in the signed-in profile: Incognito does not matter.
        let signed = ApplyInfo { session: SessionState::Matched { id: 1, name: "Lever".into(), status: SessionStatus::Ok }, ..connected };
        assert_eq!(signed.blocker(), None);
    }

    #[test]
    fn dry_run_env_values() {
        // Parsing is checked without touching the process environment of other tests.
        let parse = |v: &str| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no");
        assert!(parse("1") && parse("true") && parse("yes"));
        assert!(!parse("") && !parse("0") && !parse("false"));
    }
}
