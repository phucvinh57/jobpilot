//! A lead the user adds by hand: a pasted job link, a dropped file or pasted text becomes a row of
//! the Job list (`leads`), exactly like one discovery found, with `source_id: None`.
//!
//! ```text
//! let lead = core.add_lead(JdInput::Url(link)).await?;   // runtime::spawn: it fetches and asks the model
//! ```
//!
//! The steps, in order: read the text the way "Generate CV" does ([`ingest_with_browser`]: plain
//! fetch, headless Chrome for a page JavaScript draws); the model reads company, role and place
//! from it (`pipeline::analyze`, the step the generator starts with); the same job already in the
//! list is returned instead of added; the lead is scored like a discovered one (`discover::score`)
//! and gets its brief (`pipeline::insight`); the text goes to `<data_dir>/leads/<id>/jd.md`.
//!
//! - Nothing is promoted and no CV is written: "Prepare CV" does that when the user presses it.
//! - The hard filters do not apply: the user chose this job, so it is listed (and scored) even when
//!   discovery would have dismissed it.
//! - A link the user pasted is theirs, so it is read like the generator reads one (any public or
//!   private address); links found by sources still go through `net::http`.
//! - A lead without a link (a file, pasted text) has the address `manual:<hash of its text>`, so
//!   adding the same text twice finds the same lead. It is not a web link: the UI must not offer
//!   "Open posting" for it (check that `Lead::url` starts with `http`).

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::dedup::{canonical_url, dedup_key, Known};
use super::deps::{insight_hook, InsightHook, InsightRequest};
use super::run::{attach_jd, load_profile, MAX_POSTING_CHARS};
use super::score::score;
use super::source::Candidate;
use crate::browser::fetch::ingest_with_browser;
use crate::llm::Llm;
use crate::pipeline::analyze::{analyze, truncate_chars};
use crate::pipeline::generate::shared_llm;
use crate::pipeline::{ingest::precheck, JdInput};
use crate::prefs::Preferences;
use crate::store::{Lead, LeadState, NewLead};
use crate::Core;

/// Company shown for a posting that does not name one.
const UNKNOWN_COMPANY: &str = "Unknown company";

/// Checking for the job and inserting it happen under this lock, so the same job added twice at
/// once is stored once. Never held across an `.await`: it only covers the quick, synchronous
/// check-and-insert, so a slow page or model cannot block another add.
static SAVE: Mutex<()> = Mutex::new(());

/// The longest the part that decides whether the lead is saved may take: reading the page, the
/// model's analysis and its score. A page that hangs cannot keep the Job list's link box busy for
/// ever.
pub const ADD_TIMEOUT: Duration = Duration::from_secs(120);

/// The longest the optional company brief may take (the company pages and the model), and never
/// past [`ADD_TIMEOUT`] counted from the start of the add. When it passes, the lead is saved
/// without a brief, like discovery does when the brief fails.
pub const BRIEF_TIMEOUT: Duration = Duration::from_secs(90);

/// What the user is told when [`ADD_TIMEOUT`] passes.
const TOO_SLOW: &str = "Reading the posting took too long; open it in your browser and paste the text instead.";

/// What adding a lead needs from the outside, as values a test can replace.
#[derive(Clone)]
pub struct ManualDeps {
    /// Reads company, role and place; scores the lead.
    pub llm: Arc<dyn Llm>,
    /// The company brief, as discovery's insight hook (`DiscoverDeps::insight`).
    pub insight: InsightHook,
}

impl ManualDeps {
    /// The default AI provider (`Err` with the "no AI provider" message when there is none) and
    /// `pipeline::insight` as the brief.
    pub fn live(core: &Core) -> Result<Self> {
        let llm = shared_llm(core)?;
        Ok(Self { insight: insight_hook(core, llm.clone()), llm })
    }
}

impl Core {
    /// Add the job the user gave (a link, a file, pasted text) to the Job list and return it, with
    /// its score and brief. Idempotent: a job that is a lead already (same link, or same company
    /// and role) returns that lead unchanged, whatever its state (a dismissed lead stays dismissed:
    /// check [`Lead::state`]). Never creates an application and never writes a CV.
    ///
    /// Takes seconds to about a minute (the page, the model twice, the company pages): run it on
    /// the global runtime. Errors the user can act on: no AI provider, no profile, a bad link or an
    /// unreadable file, a page that needs a login or captcha (`open the link in your browser and
    /// paste the text`), no Chrome for a page that needs one, a network failure (retry), text that
    /// has no job title, and a job that is an application already.
    pub async fn add_lead(&self, input: JdInput) -> Result<Lead> {
        let deps = ManualDeps::live(self)?;
        let paths = self.paths.clone();
        self.add_lead_with(&deps, input, move |input| async move { ingest_with_browser(&paths, input).await }).await
    }

    /// [`add_lead`](Self::add_lead) with explicit [`ManualDeps`] and the function that reads the
    /// input (the default is [`ingest_with_browser`]).
    pub async fn add_lead_with<F, Fut>(&self, deps: &ManualDeps, input: JdInput, read: F) -> Result<Lead>
    where
        F: FnOnce(JdInput) -> Fut,
        Fut: Future<Output = Result<crate::pipeline::JdText>>,
    {
        self.add_lead_within(deps, input, read, Limits { add: ADD_TIMEOUT, brief: BRIEF_TIMEOUT }).await
    }

    /// [`add_lead_with`](Self::add_lead_with) with its time limits given. Nothing is saved when
    /// `limits.add` passes (the save itself has no `.await`); a brief that passes `limits.brief`
    /// is left out and the lead is saved without it.
    async fn add_lead_within<F, Fut>(&self, deps: &ManualDeps, input: JdInput, read: F, limits: Limits) -> Result<Lead>
    where
        F: FnOnce(JdInput) -> Fut,
        Fut: Future<Output = Result<crate::pipeline::JdText>>,
    {
        precheck(&input)?;
        // A missing profile is a setup problem: say so before anything is fetched.
        let profile = load_profile(self)?;
        let prefs = self.store.preferences()?;

        // The same link again costs nothing.
        let asked = match &input {
            JdInput::Url(url) | JdInput::Html { url, .. } => canonical_url(url),
            _ => None,
        };
        if let Some(lead) = self.known_lead(asked.as_deref())? {
            return Ok(lead);
        }

        self.read_and_save(deps, input, read, &profile, &prefs, limits).await
    }

    /// The slow part of an add: read the input, ask the model, score, brief, save. The deadline of
    /// `limits.add`, counted from the start of this call, covers the reading, the analysis and the
    /// score (what decides whether the lead is saved). The brief runs beside the score on its own
    /// clock (`limits.brief`) but is cut at the same deadline, and is dropped when the score fails.
    async fn read_and_save<F, Fut>(
        &self,
        deps: &ManualDeps,
        input: JdInput,
        read: F,
        profile: &str,
        prefs: &Preferences,
        limits: Limits,
    ) -> Result<Lead>
    where
        F: FnOnce(JdInput) -> Fut,
        Fut: Future<Output = Result<crate::pipeline::JdText>>,
    {
        let deadline = tokio::time::Instant::now() + limits.add;
        let jd = by(deadline, read(input)).await?;
        let text = truncate_chars(jd.text.trim(), MAX_POSTING_CHARS).to_string();
        let web_url = jd.url.as_deref().or(jd.source.as_deref()).and_then(canonical_url);
        let url = web_url.clone().unwrap_or_else(|| text_url(&text));
        // The page may have moved (redirects), and the same text may have been added before.
        if let Some(lead) = self.known_lead(Some(&url))? {
            return Ok(lead);
        }

        // Company, role and place: the model reads them (never the file's path, only the link).
        let found = by(deadline, analyze(&*deps.llm, &text, web_url.as_deref())).await?;
        let (role, location) = (found.role.trim().to_string(), found.location.trim().to_string());
        if role.is_empty() {
            bail!("no job title was found in this text: is it a job description?");
        }
        let company = match found.company.trim() {
            "" => UNKNOWN_COMPANY.to_string(),
            name => name.to_string(),
        };
        // A company that is not named has no key: "Unknown company - Engineer" would match any other.
        let key = dedup_key(found.company.trim(), &role);

        let candidate = Candidate { url: url.clone(), title: role.clone(), company: company.clone(), location, snippet: String::new() };
        if let Some(lead) = self.existing(&url, &key)? {
            return self.repaired(lead, &candidate, &text);
        }

        // Scoring and the brief do not depend on each other. The brief is optional, like in a run:
        // a failure or a brief that is too slow leaves the lead without one.
        let request = InsightRequest {
            url: web_url.clone().unwrap_or_default(),
            company: company.clone(),
            role: role.clone(),
            jd_text: text.clone(),
        };
        // The brief has its own limit, but it may not outlive the add's deadline either.
        let brief_by = (tokio::time::Instant::now() + limits.brief).min(deadline);
        let brief = async {
            match tokio::time::timeout_at(brief_by, (deps.insight)(request)).await {
                Ok(Ok(json)) => json.filter(|j| serde_json::from_str::<Value>(j).is_ok()),
                Ok(Err(_)) => {
                    tracing::warn!("insight failed, the lead is saved without it");
                    None
                }
                Err(_) => {
                    tracing::warn!(seconds = limits.brief.as_secs(), "insight took too long, the lead is saved without it");
                    None
                }
            }
        };
        // `try_join!` drops the brief as soon as the score fails or times out: nobody waits for a
        // brief of a lead that is not saved.
        let (scored, insight_json) = tokio::try_join!(by(deadline, score(&*deps.llm, prefs, profile, &candidate, &text)), async {
            Ok::<_, anyhow::Error>(brief.await)
        })?;

        let lead = {
            let _one_at_a_time = SAVE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(lead) = self.existing(&url, &key)? {
                return self.repaired(lead, &candidate, &text);
            }
            let new = NewLead {
                source_id: None,
                url: url.clone(),
                dedup_key: key,
                company: company.clone(),
                role,
                location: candidate.location.clone(),
                jd_path: None,
                score: scored.total,
                score_json: Some(scored.to_json()?),
                insight_json,
                state: LeadState::New,
                dismiss_reason: None,
            };
            self.store.add_lead(&new)?
        };
        attach_jd(&self.store, &self.paths, lead.id, &company, &candidate, &text)?;
        tracing::info!(lead_id = lead.id, scored = lead.score.is_some(), brief = lead.insight_json.is_some(), "lead added by hand");
        self.store.get_lead(lead.id)?.context("the lead vanished after it was saved")
    }

    /// The lead with this address, when its text is on disk (nothing is left to do for it); an
    /// error when the address is an application's.
    fn known_lead(&self, url: Option<&str>) -> Result<Option<Lead>> {
        let Some(url) = url else { return Ok(None) };
        let found = self.existing(url, "")?.filter(has_text);
        if let Some(lead) = &found {
            tracing::info!(lead_id = lead.id, "lead added by hand already exists");
        }
        Ok(found)
    }

    /// The lead for this posting, if there is one; an error when it is an application already
    /// (the user has that job in hand: a lead for it would only be prepared a second time).
    fn existing(&self, url: &str, key: &str) -> Result<Option<Lead>> {
        if let Some(lead) = self.store.find_lead(url, key)? {
            return Ok(Some(lead));
        }
        if Known::load(&self.store)?.has_application(url, key) {
            bail!("you already have an application for this job");
        }
        Ok(None)
    }

    /// `lead` is the job being added: make sure its text is on disk (a crash after the row was
    /// inserted can leave it without) and return it.
    fn repaired(&self, lead: Lead, candidate: &Candidate, text: &str) -> Result<Lead> {
        if has_text(&lead) {
            tracing::info!(lead_id = lead.id, "lead added by hand already exists");
            return Ok(lead);
        }
        attach_jd(&self.store, &self.paths, lead.id, &lead.company, candidate, text)?;
        self.store.get_lead(lead.id)?.context("the lead vanished")
    }
}

/// How long the two parts of an add may take.
#[derive(Clone, Copy)]
struct Limits {
    /// Reading the page, analysis and score: [`ADD_TIMEOUT`].
    add: Duration,
    /// The brief: [`BRIEF_TIMEOUT`].
    brief: Duration,
}

/// `fut`, or the "too slow" error when `deadline` passes first.
async fn by<T>(deadline: tokio::time::Instant, fut: impl Future<Output = Result<T>>) -> Result<T> {
    match tokio::time::timeout_at(deadline, fut).await {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!("adding a lead by hand took too long");
            bail!(TOO_SLOW)
        }
    }
}

/// The posting text of the lead is saved.
fn has_text(lead: &Lead) -> bool {
    lead.jd_path.as_deref().is_some_and(|p| p.is_file())
}

/// The address of a lead that has no link: `manual:` and 16 hex digits of the text's hash.
fn text_url(text: &str) -> String {
    let digest = Sha256::digest(text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("manual:{hex}")
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use serde_json::json;

    use super::*;
    use crate::browser::fetch::ingest_with_fallback;
    use crate::discover::lead_dir;
    use crate::llm::fake::FakeLlm;
    use crate::pipeline::ingest::tests::{serve, JOB_HTML};
    use crate::pipeline::ingest;
    use crate::prefs::Preferences;
    use crate::store::NewApplication;
    use crate::{Paths, Secrets};

    const BRIEF: &str = r#"{"domain": "Robotics"}"#;
    const JOB_TEXT: &str = "Acme Robotics is hiring a Senior Rust Engineer for the platform team. You will build backend services in Rust and Go and operate Kubernetes on AWS.";

    fn analysis(company: &str, role: &str) -> Value {
        json!({
            "company": company, "role": role, "slug": "", "location": "Ho Chi Minh City", "apply_url": "",
            "must_have": ["Rust"], "nice_to_have": [], "key_terms": ["Rust"], "seniority": "senior"
        })
    }

    fn scores() -> Value {
        json!({
            "skills": {"score": 80, "known": true, "reason": "matches"},
            "domain": {"score": 60, "known": true, "reason": "ok"},
            "location": {"score": 100, "known": true, "reason": "remote"},
            "salary": {"score": 0, "known": false, "reason": "not stated"},
            "seniority": {"score": 70, "known": true, "reason": "fits"}
        })
    }

    /// The replies of one lead that is added: the analysis, then the score.
    fn one_job() -> Vec<Value> {
        vec![analysis("Acme Robotics", "Senior Rust Engineer"), scores()]
    }

    struct Harness {
        _dir: tempfile::TempDir,
        core: Core,
        llm: Arc<FakeLlm>,
        deps: ManualDeps,
        /// The briefs asked for.
        asked: Arc<Mutex<Vec<InsightRequest>>>,
        /// What the brief hook answers.
        brief: Arc<Mutex<Result<Option<String>, String>>>,
    }

    impl Harness {
        fn new(replies: impl IntoIterator<Item = Value>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
            let profile = dir.path().join("profile.yaml");
            std::fs::write(&profile, "basics: {name: Ada, headline: Rust engineer}\nskills:\n  - {category: Languages, items: [Rust, Go]}\n").unwrap();
            core.set_profile_path(&profile).unwrap();
            let llm = Arc::new(FakeLlm::with_replies(replies));
            let asked: Arc<Mutex<Vec<InsightRequest>>> = Arc::default();
            let brief: Arc<Mutex<Result<Option<String>, String>>> = Arc::new(Mutex::new(Ok(Some(BRIEF.to_string()))));
            let (log, answer) = (asked.clone(), brief.clone());
            let insight: InsightHook = Arc::new(move |req| {
                log.lock().unwrap().push(req);
                let answer = answer.lock().unwrap().clone();
                async move { answer.map_err(|m| anyhow::anyhow!(m)) }.boxed()
            });
            let deps = ManualDeps { llm: llm.clone(), insight };
            Self { _dir: dir, core, llm, deps, asked, brief }
        }

        async fn add(&self, input: JdInput) -> Result<Lead> {
            self.core.add_lead_with(&self.deps, input, |i| async move { ingest(i).await }).await
        }

        fn leads(&self) -> Vec<Lead> {
            [LeadState::New, LeadState::Dismissed, LeadState::Promoted]
                .into_iter()
                .flat_map(|s| self.core.store.list_leads(s).unwrap())
                .collect()
        }
    }

    #[tokio::test]
    async fn a_pasted_link_becomes_a_scored_lead_with_a_brief_and_never_an_application() {
        let h = Harness::new(one_job());
        let link = serve(200, "text/html", JOB_HTML.to_string()).await;
        let lead = h.add(JdInput::Url(link.clone())).await.unwrap();

        assert_eq!((lead.company.as_str(), lead.role.as_str()), ("Acme Robotics", "Senior Rust Engineer"));
        assert_eq!(lead.location, "Ho Chi Minh City");
        assert_eq!(lead.source_id, None);
        assert_eq!(lead.url, link, "the canonical link");
        assert_eq!(lead.dedup_key, "acme-robotics-senior-rust-engineer");
        assert_eq!((lead.state, lead.app_id), (LeadState::New, None));
        assert_eq!(lead.score, Some(77.1), "weighted by the default weights, salary not judged");
        assert!(lead.score_json.as_deref().is_some_and(|j| j.contains("\"skills\"")));
        assert_eq!(lead.insight_json.as_deref(), Some(BRIEF));
        let asked = h.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!((asked[0].url.as_str(), asked[0].company.as_str()), (link.as_str(), "Acme Robotics"));
        assert!(asked[0].jd_text.contains("Kubernetes clusters on AWS"));

        // The text is where "Prepare CV" reads it, in the layout of every lead.
        let jd_path = lead.jd_path.clone().expect("jd path");
        assert_eq!(jd_path, lead_dir(&h.core.paths, lead.id).join("jd.md"));
        let jd = std::fs::read_to_string(&jd_path).unwrap();
        assert!(jd.starts_with(&format!("# Acme Robotics \u{2014} Senior Rust Engineer\nSource: {link}\n\n")), "{jd}");
        assert!(jd.contains("Kubernetes clusters on AWS"));

        // A lead, nothing more: no application, no CV.
        assert!(h.core.store.list_applications().unwrap().is_empty());
        let files: Vec<_> = std::fs::read_dir(lead_dir(&h.core.paths, lead.id)).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, ["jd.md"]);
        assert_eq!(h.llm.calls().len(), 2, "the analysis and the score");
        assert_eq!(h.core.store.list_queue_leads().unwrap().len(), 1, "it is in the Job list");
    }

    #[tokio::test]
    async fn adding_the_same_job_again_returns_the_same_lead() {
        let mut replies = one_job();
        replies.push(analysis("Acme Robotics Inc.", "Senior Rust Engineer (Remote)"));
        let h = Harness::new(replies);
        let link = serve(200, "text/html", JOB_HTML.to_string()).await;
        let first = h.add(JdInput::Url(link.clone())).await.unwrap();

        // The same link, spelled with tracking noise: no fetch, no model call, no second brief.
        let again = h.add(JdInput::Url(format!("{link}?utm_source=x#top"))).await.unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(h.llm.calls().len(), 2);

        // The same job from another place (pasted text): the model reads it, the key matches.
        let pasted = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap();
        assert_eq!(pasted.id, first.id, "same company and role");
        assert_eq!(h.llm.calls().len(), 3, "only the analysis");
        assert_eq!(h.leads().len(), 1);
        assert_eq!(h.asked.lock().unwrap().len(), 1);
        assert!(h.core.store.list_applications().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_dismissed_lead_is_returned_as_it_is() {
        let h = Harness::new(one_job());
        let link = serve(200, "text/html", JOB_HTML.to_string()).await;
        let first = h.add(JdInput::Url(link.clone())).await.unwrap();
        h.core.store.dismiss_lead(first.id, "not for me").unwrap();
        let again = h.add(JdInput::Url(link)).await.unwrap();
        assert_eq!((again.id, again.state), (first.id, LeadState::Dismissed));
    }

    #[tokio::test]
    async fn pasted_text_and_a_file_get_a_manual_address_and_the_same_text_is_the_same_lead() {
        let h = Harness::new(one_job());
        let lead = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap();
        assert!(lead.url.starts_with("manual:") && lead.url.len() == "manual:".len() + 16, "{}", lead.url);
        assert_eq!(lead.company, "Acme Robotics");
        assert_eq!(h.asked.lock().unwrap()[0].url, "", "no address to read company pages from");
        assert!(std::fs::read_to_string(lead.jd_path.as_ref().unwrap()).unwrap().contains("operate Kubernetes on AWS"));

        // The same words, spaced and cased differently, as a file: found by its address, no model call.
        let file = h._dir.path().join("jd.md");
        std::fs::write(&file, format!("{}\n\n", JOB_TEXT.replace(' ', "  "))).unwrap();
        let again = h.add(JdInput::File(file)).await.unwrap();
        assert_eq!(again.id, lead.id);
        assert_eq!(h.llm.calls().len(), 2);
        assert_eq!(h.leads().len(), 1);
        assert!(h.core.store.list_applications().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_file_path_is_never_sent_to_the_model() {
        let h = Harness::new(one_job());
        let file = h._dir.path().join("secret-name.txt");
        std::fs::write(&file, JOB_TEXT).unwrap();
        h.add(JdInput::File(file)).await.unwrap();
        let calls = h.llm.calls();
        assert!(calls.iter().all(|c| !c.user.contains("secret-name")), "{calls:?}");
    }

    /// Answers by the question, not by the order: two calls at once may ask in any order.
    struct ByQuestion;

    impl Llm for ByQuestion {
        fn complete_json<'a>(&'a self, _system: &'a str, _user: &'a str, schema: &'a Value) -> crate::llm::LlmFuture<'a, Value> {
            let reply = if schema["properties"].get("company").is_some() { analysis("Acme Robotics", "Senior Rust Engineer") } else { scores() };
            Box::pin(std::future::ready(Ok(reply)))
        }

        fn list_models(&self) -> crate::llm::LlmFuture<'_, Vec<String>> {
            Box::pin(std::future::ready(Ok(Vec::new())))
        }
    }

    #[tokio::test]
    async fn the_same_job_added_at_the_same_time_is_stored_once() {
        let mut h = Harness::new([]);
        h.deps.llm = Arc::new(ByQuestion);
        let link = serve(200, "text/html", JOB_HTML.to_string()).await;
        let (a, b) = tokio::join!(h.add(JdInput::Url(link.clone())), h.add(JdInput::Url(link)));
        assert_eq!(a.unwrap().id, b.unwrap().id);
        assert_eq!(h.leads().len(), 1);
    }

    #[tokio::test]
    async fn a_job_the_user_has_an_application_for_is_refused_and_nothing_is_stored() {
        let h = Harness::new(one_job());
        let link = serve(200, "text/html", JOB_HTML.to_string()).await;
        let app = |source: Option<String>| NewApplication {
            slug: "acme-rust".into(),
            dir: h.core.paths.application_dir("acme-rust"),
            company: "Acme Robotics".into(),
            role: "Senior Rust Engineer".into(),
            source,
            ..Default::default()
        };
        h.core.store.create_application(&app(Some(link.clone()))).unwrap();
        let err = h.add(JdInput::Url(link)).await.unwrap_err();
        assert!(format!("{err:#}").contains("already have an application"), "{err:#}");
        assert!(h.llm.calls().is_empty(), "refused before the page was read");

        // The same job from pasted text: found by company and role after the model read it.
        let err = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap_err();
        assert!(format!("{err:#}").contains("already have an application"), "{err:#}");
        assert!(h.leads().is_empty());
        assert_eq!(h.core.store.list_applications().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_hard_filters_do_not_apply_to_a_job_the_user_chose() {
        let h = Harness::new(one_job());
        h.core.save_preferences(Preferences { exclude_companies: vec!["Acme".into()], ..Default::default() }).unwrap();
        let lead = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap();
        assert_eq!(lead.state, LeadState::New);
        assert!(lead.score.is_some());
    }

    #[tokio::test]
    async fn a_failing_brief_leaves_the_lead_without_one_and_a_brief_that_is_not_json_is_dropped() {
        let h = Harness::new([one_job(), vec![analysis("Globex", "SRE"), scores()]].concat());
        *h.brief.lock().unwrap() = Err("pages unreachable".into());
        let lead = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap();
        assert_eq!(lead.insight_json, None);
        assert!(lead.score.is_some(), "scored all the same");

        *h.brief.lock().unwrap() = Ok(Some("not json".into()));
        let other = h.add(JdInput::Text("Globex needs an SRE who knows Linux, networks, databases and on-call work.".into())).await.unwrap();
        assert_eq!((other.company.as_str(), other.insight_json), ("Globex", None));
    }

    #[tokio::test]
    async fn text_without_a_job_title_is_refused_and_a_missing_company_is_not() {
        let h = Harness::new([vec![analysis("Acme", "  ")], vec![analysis("", "Backend Engineer"), scores(), analysis("", "Backend Engineer"), scores()]].concat());
        let err = h.add(JdInput::Text("Just a long enough note about nothing in particular, really.".into())).await.unwrap_err();
        assert!(format!("{err:#}").contains("no job title"), "{err:#}");
        assert!(h.leads().is_empty());
        assert!(!h.core.paths.data_dir.join("leads").exists() || std::fs::read_dir(h.core.paths.data_dir.join("leads")).unwrap().next().is_none());

        let a = h.add(JdInput::Text("A small shop wants a backend engineer for payments, Rust and Postgres.".into())).await.unwrap();
        assert_eq!((a.company.as_str(), a.dedup_key.as_str()), ("Unknown company", ""));
        // No company means no key: another unnamed company with the same title is another job.
        let b = h.add(JdInput::Text("Another shop wants a backend engineer for logistics, Go and Redis.".into())).await.unwrap();
        assert_ne!(a.id, b.id);
    }

    #[tokio::test]
    async fn setup_problems_come_before_any_work() {
        // No AI provider.
        let h = Harness::new(one_job());
        let err = h.core.add_lead(JdInput::Text(JOB_TEXT.into())).await.unwrap_err();
        assert_eq!(crate::health::classify(&format!("{err:#}")), crate::health::Problem::NoProvider, "{err:#}");

        // No profile: nothing is fetched or asked.
        h.core.set_profile_path(&h._dir.path().join("missing.yaml")).unwrap();
        let err = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap_err();
        assert!(format!("{err:#}").contains("cannot read the profile"), "{err:#}");
        assert!(h.llm.calls().is_empty());

        // Bad input is refused before the profile or the network.
        for bad in [JdInput::Url("not a url".into()), JdInput::Url("ftp://x.test/job".into()), JdInput::Text("short".into()), JdInput::File("/definitely/missing.md".into())] {
            assert!(h.add(bad).await.is_err());
        }
        assert!(h.leads().is_empty());
    }

    #[tokio::test]
    async fn a_page_that_needs_a_browser_is_read_through_the_read_hook() {
        let h = Harness::new(one_job());
        let spa = serve(200, "text/html", "<html><body><div id=root></div><script src=app.js></script></body></html>".to_string()).await;
        let lead = h
            .core
            .add_lead_with(&h.deps, JdInput::Url(spa.clone()), |i| ingest_with_fallback(i, |_| async { Ok(JOB_HTML.to_string()) }))
            .await
            .unwrap();
        assert_eq!(lead.url, spa);
        assert!(std::fs::read_to_string(lead.jd_path.unwrap()).unwrap().contains("Kubernetes clusters on AWS"));

        // A page that shows a wall instead of a posting fails with a message the user can act on.
        let walled = serve(403, "text/html", "blocked".to_string()).await;
        let err = h
            .core
            .add_lead_with(&h.deps, JdInput::Url(walled), |i| ingest_with_fallback(i, |_| async { Err(anyhow::anyhow!("The site asked for a login instead of the job description: open the link in your browser and paste the text or upload the file")) }))
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("paste the text"), "{err:#}");
        assert_eq!(h.leads().len(), 1);
    }

    #[tokio::test]
    async fn a_lead_whose_text_was_lost_gets_it_back_when_it_is_added_again() {
        let h = Harness::new([one_job(), vec![analysis("Acme Robotics", "Senior Rust Engineer")]].concat());
        let lead = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap();
        let path = lead.jd_path.clone().unwrap();
        std::fs::remove_file(&path).unwrap();
        let again = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap();
        assert_eq!(again.id, lead.id);
        assert!(std::fs::read_to_string(&path).unwrap().contains("operate Kubernetes on AWS"));
        assert_eq!(h.leads().len(), 1);
    }

    fn limits(add_ms: u64, brief_ms: u64) -> Limits {
        Limits { add: Duration::from_millis(add_ms), brief: Duration::from_millis(brief_ms) }
    }

    #[tokio::test]
    async fn a_page_that_never_answers_gives_up_with_a_clear_message_and_frees_the_lock() {
        let h = Harness::new(one_job());
        let link = serve(200, "text/html", JOB_HTML.to_string()).await;
        // The read hangs: the add stops at the limit, saves nothing and says what to do.
        let err = h
            .core
            .add_lead_within(&h.deps, JdInput::Url(link.clone()), |_| std::future::pending(), limits(50, 5_000))
            .await
            .unwrap_err();
        assert_eq!(format!("{err:#}"), TOO_SLOW);
        assert!(h.leads().is_empty());
        assert!(h.llm.calls().is_empty());
        // The save lock is free again: the same link added right after works.
        let lead = h.add(JdInput::Url(link)).await.unwrap();
        assert_eq!(lead.role, "Senior Rust Engineer");
        assert!(ADD_TIMEOUT >= Duration::from_secs(60), "long enough for a slow page and two model calls");
        assert!(BRIEF_TIMEOUT >= Duration::from_secs(30), "long enough for the company pages and a model call");
    }

    /// Reads the posting, then never scores it.
    struct HangsOnScore;

    impl Llm for HangsOnScore {
        fn complete_json<'a>(&'a self, _system: &'a str, _user: &'a str, schema: &'a Value) -> crate::llm::LlmFuture<'a, Value> {
            if schema["properties"].get("company").is_some() {
                Box::pin(std::future::ready(Ok(analysis("Acme Robotics", "Senior Rust Engineer"))))
            } else {
                Box::pin(std::future::pending())
            }
        }

        fn list_models(&self) -> crate::llm::LlmFuture<'_, Vec<String>> {
            Box::pin(std::future::ready(Ok(Vec::new())))
        }
    }

    #[tokio::test]
    async fn a_score_that_never_comes_gives_up_and_saves_nothing() {
        let mut h = Harness::new([]);
        h.deps.llm = Arc::new(HangsOnScore);
        // The brief hangs too: nobody waits for it once the score has given up.
        h.deps.insight = Arc::new(|_| std::future::pending().boxed());
        let started = std::time::Instant::now();
        let err = h.core.add_lead_within(&h.deps, JdInput::Text(JOB_TEXT.into()), |i| async move { ingest(i).await }, limits(300, 5_000)).await.unwrap_err();
        assert_eq!(format!("{err:#}"), TOO_SLOW, "the score is part of what decides whether the lead is saved");
        assert!(started.elapsed() < Duration::from_secs(3), "it did not wait for the brief's own limit: {:?}", started.elapsed());
        assert!(h.leads().is_empty());
    }

    #[tokio::test]
    async fn a_score_that_fails_does_not_wait_for_the_brief() {
        // Only the analysis has a reply: the score call fails at once.
        let mut h = Harness::new([analysis("Acme Robotics", "Senior Rust Engineer")]);
        h.deps.insight = Arc::new(|_| std::future::pending().boxed());
        let started = std::time::Instant::now();
        let result = h.core.add_lead_within(&h.deps, JdInput::Text(JOB_TEXT.into()), |i| async move { ingest(i).await }, limits(30_000, 30_000)).await;
        assert!(result.is_err());
        assert_ne!(format!("{:#}", result.unwrap_err()), TOO_SLOW, "the score failed, it did not time out");
        assert!(started.elapsed() < Duration::from_secs(5), "it did not wait for the brief: {:?}", started.elapsed());
        assert!(h.leads().is_empty());
    }

    #[tokio::test]
    async fn a_brief_that_is_too_slow_is_left_out_and_the_lead_is_saved() {
        let mut h = Harness::new(one_job());
        h.deps.insight = Arc::new(|_| std::future::pending().boxed());
        let started = std::time::Instant::now();
        let lead = h.core.add_lead_within(&h.deps, JdInput::Text(JOB_TEXT.into()), |i| async move { ingest(i).await }, limits(30_000, 100)).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "it did not wait for the add limit");
        assert_eq!(lead.insight_json, None, "saved without a brief, as when it fails");
        assert!(lead.score.is_some());
        assert_eq!(h.leads().len(), 1);
        assert!(std::fs::read_to_string(lead.jd_path.unwrap()).unwrap().contains("operate Kubernetes on AWS"));
    }

    #[tokio::test]
    async fn the_brief_is_cut_at_the_add_limit_even_when_its_own_limit_is_longer() {
        let mut h = Harness::new(one_job());
        // Reading and scoring take no time; the brief would take longer than the add limit and
        // is well within its own.
        h.deps.insight = Arc::new(|_| std::future::pending().boxed());
        let started = std::time::Instant::now();
        let lead = h.core.add_lead_within(&h.deps, JdInput::Text(JOB_TEXT.into()), |i| async move { ingest(i).await }, limits(200, 30_000)).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "the add limit bounds the whole add: {:?}", started.elapsed());
        assert_eq!(lead.insight_json, None, "saved without a brief");
        assert!(lead.score.is_some());
    }

    #[tokio::test]
    async fn the_brief_gets_what_is_left_of_the_add_limit_after_a_slow_read() {
        let mut h = Harness::new(one_job());
        h.deps.insight = Arc::new(|_| {
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok(Some(BRIEF.to_string()))
            }
            .boxed()
        });
        // The read takes 300 ms of the 500 ms; the brief needs 300 ms more than is left.
        let slow_read = |i| async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            ingest(i).await
        };
        let lead = h.core.add_lead_within(&h.deps, JdInput::Text(JOB_TEXT.into()), slow_read, limits(500, 30_000)).await.unwrap();
        assert_eq!(lead.insight_json, None, "the clock started with the read");

        // The same brief within the limit is kept.
        let mut h = Harness::new(one_job());
        h.deps.insight = Arc::new(|_| {
            async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(Some(BRIEF.to_string()))
            }
            .boxed()
        });
        let lead = h.core.add_lead_within(&h.deps, JdInput::Text(JOB_TEXT.into()), |i| async move { ingest(i).await }, limits(5_000, 5_000)).await.unwrap();
        assert_eq!(lead.insight_json.as_deref(), Some(BRIEF));
    }

    #[tokio::test]
    async fn the_posting_text_is_replaced_whole_so_a_reader_never_sees_it_cut_short() {
        let h = Harness::new(one_job());
        let lead = h.add(JdInput::Text(JOB_TEXT.into())).await.unwrap();
        let path = lead.jd_path.clone().unwrap();
        let candidate = Candidate { url: lead.url.clone(), title: "Engineer".into(), company: "Acme".into(), location: String::new(), snippet: String::new() };
        let text = "word ".repeat(20_000);
        attach_jd(&h.core.store, &h.core.paths, lead.id, "Acme", &candidate, &text).unwrap();
        let whole = std::fs::read_to_string(&path).unwrap().len();
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            let writers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        for _ in 0..60 {
                            attach_jd(&h.core.store, &h.core.paths, lead.id, "Acme", &candidate, &text).unwrap();
                        }
                    })
                })
                .collect();
            let reader = scope.spawn(|| {
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    assert_eq!(std::fs::read_to_string(&path).unwrap().len(), whole, "a reader saw a half-written jd.md");
                }
            });
            for w in writers {
                w.join().unwrap();
            }
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            reader.join().unwrap();
        });
        let files: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, ["jd.md"], "no temporary file is left behind");
    }

    #[test]
    fn the_address_of_a_text_ignores_spacing_and_case_only() {
        assert_eq!(text_url("Hello  World\n"), text_url("hello world"));
        assert_ne!(text_url("hello world"), text_url("hello worlds"));
    }
}
