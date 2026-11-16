//! The two discovery graphs.
//!
//! ```text
//! discovery   plan -> sources (one source_run thread per source, run_each) -> summary
//!
//! source_run  search -> challenge -+-> wait_human -> (search | fetch)      (a login, captcha, block)
//!                                  +-> give_up -> END                      (third challenge in a row)
//!                                  `-> dedup -> next_candidate -+-> fetch -> challenge_page -> filter
//!                                                               +-> search (next page)           |
//!                                                               `-> finish -> END       dismiss <+> fan
//!                                                                                                  |
//!                                                                      next_candidate <- save_lead <- score + insight
//! ```
//!
//! A source is its own thread (`discover:<run>:<source id>`), so one that waits at `wait_human`
//! never holds back the others; the run is finished when every source is done, waiting or failed.
//! One source failing ends only its own thread; "Retry" ([`Core::discover_resume`]) re-runs the
//! node that failed. Logs carry ids and counts only.
//!
//! [`Core::discover_resume`]: crate::engine::Core::discover_resume

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use super::dedup::{canonical_url, dedup_key, Known};
use super::deps::{ChallengeCheck, CheckStage, DiscoverDeps, FetchRequest, InsightRequest};
use super::limits::{MAX_CHALLENGES_IN_A_ROW, SCAN_FACTOR};
use super::score::{compact_profile, hard_filter, ineligible_reason, score};
use super::source::boards::no_postings_status;
use super::source::{one_line, Candidate, JobSource, SearchCtx, SNIPPET_CHARS};
use super::state::{
    parse_source_thread, source_thread, wait_label, DiscoveryState, RunSummary, SourceResult,
    SourceState, SourceStatus, DISCOVERY_GRAPH, LAST_RUN_KEY, SOURCE_GRAPH,
};
use crate::engine::browser::challenge::ChallengeFound;
use crate::engine::browser::{BridgeError, INCOGNITO_NOT_ALLOWED, NOT_CONNECTED};
use crate::engine::graph::nodes::{self, Challenge, ChallengeKind};
use crate::engine::graph::{GraphDef, Graphs, Limits, Outcome, END, START};
use crate::engine::health::{classify, Problem};
use crate::engine::pipeline::analyze::truncate_chars;
use crate::engine::pipeline::ingest::NeedsBrowser;
use crate::engine::redact::redact;
use crate::engine::store::{LeadState, NewLead, Source, SourcePurpose, Store};
use crate::engine::{Core, Paths};

/// A posting with less text than this is not a posting (a bot page, an empty app shell).
const MIN_POSTING_CHARS: usize = 40;
/// Longest posting text kept in the state of a run (and so in every checkpoint).
pub(super) const MAX_POSTING_CHARS: usize = 40_000;
/// How much of a fetched posting a challenge check sees.
const CHECK_TEXT_CHARS: usize = 2_000;
/// The `sources` node waits for every source, which can take hours when sources wait for the user
/// or the model is slow.
const SOURCES_TIMEOUT: Duration = Duration::from_secs(6 * 3600);
/// Steps of one source thread over its life: about eight per posting.
const SOURCE_MAX_STEPS: usize = 5_000;

/// Everything the nodes share.
pub(crate) struct Shared {
    store: Store,
    paths: Paths,
    graphs: Graphs,
    deps: DiscoverDeps,
    /// The compact profile for scoring.
    profile: String,
    /// One adapter per source for the life of a run, so its listing cache serves every page.
    adapters: Mutex<HashMap<i64, Arc<dyn JobSource>>>,
    /// Held while a lead is checked and inserted, so two sources finding the same job at the
    /// same time save it once.
    save: Mutex<()>,
}

impl Shared {
    fn source(&self, id: i64) -> Result<Source> {
        self.store.get_source(id)?.with_context(|| format!("source {id} no longer exists"))
    }

    /// The adapter this run made for a source, if it made one.
    fn adapter_of(&self, source_id: i64) -> Option<Arc<dyn JobSource>> {
        self.adapters.lock().unwrap_or_else(|e| e.into_inner()).get(&source_id).cloned()
    }

    fn adapter(&self, source: &Source) -> Result<Arc<dyn JobSource>> {
        let mut adapters = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = adapters.get(&source.id) {
            return Ok(found.clone());
        }
        let made = (self.deps.sources)(source)?;
        adapters.insert(source.id, made.clone());
        Ok(made)
    }
}

/// The two graphs over one set of dependencies.
pub(crate) struct Runner {
    pub(crate) source: GraphDef<SourceState>,
    pub(crate) discovery: GraphDef<DiscoveryState>,
}

impl Runner {
    pub(crate) fn new(core: &Core, deps: DiscoverDeps) -> Result<Self> {
        let shared = Arc::new(Shared {
            store: core.store.clone(),
            paths: core.paths.clone(),
            graphs: core.graphs.clone(),
            profile: load_profile(core)?,
            deps,
            adapters: Mutex::new(HashMap::new()),
            save: Mutex::new(()),
        });
        let source = source_graph(&shared);
        let discovery = discovery_graph(&shared, source.clone());
        Ok(Self { source, discovery })
    }
}

/// The stored profile, compacted for scoring (the `NoProfile` error when there is none).
pub(super) fn load_profile(core: &Core) -> Result<String> {
    Ok(compact_profile(&core.require_profile()?.cv))
}

// ---------------------------------------------------------------- the source graph

/// A node over [`Shared`] and the state, as `GraphDef::node` wants it.
fn node<F, Fut>(sh: &Arc<Shared>, f: F) -> impl Fn(SourceState) -> Fut + Send + Sync + 'static
where
    F: Fn(Arc<Shared>, SourceState) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<SourceState>> + Send + 'static,
{
    let sh = sh.clone();
    move |s| f(sh.clone(), s)
}

/// After a challenge was found: stop for the day, wait for the user, or go on.
fn after_check(s: &SourceState, clear: &'static str) -> &'static str {
    match (s.challenge.is_some(), s.strikes >= MAX_CHALLENGES_IN_A_ROW) {
        (true, true) => "give_up",
        (true, false) => "wait_human",
        _ => clear,
    }
}

fn source_graph(sh: &Arc<Shared>) -> GraphDef<SourceState> {
    GraphDef::new(SOURCE_GRAPH)
        .append("candidates")
        .limits(Limits { max_steps: SOURCE_MAX_STEPS, ..Limits::default() })
        .node("search", node(sh, search))
        .node("challenge", node(sh, |sh, s| check(sh, s, CheckStage::Search)))
        .wait("wait_human", nodes::challenge::<SourceState>)
        .node("give_up", node(sh, give_up))
        .node("dedup", node(sh, dedup))
        .node("next_candidate", node(sh, next_candidate))
        .node("fetch", node(sh, fetch))
        .node("challenge_page", node(sh, |sh, s| check(sh, s, CheckStage::Posting)))
        .node("filter", node(sh, filter))
        .node("fan", |s| std::future::ready(Ok(s)))
        .node("score", node(sh, score_node))
        .node("insight", node(sh, insight_node))
        .node("save_lead", node(sh, save_lead))
        .node("dismiss", node(sh, dismiss))
        .node("finish", node(sh, finish))
        .edge(START, "search")
        .edge("search", "challenge")
        .route("challenge", |s| after_check(s, "dedup"), &["give_up", "wait_human", "dedup"])
        // "Continue" goes back to where the challenge was met.
        .route("wait_human", |s| if s.current.is_some() { "fetch" } else { "search" }, &["fetch", "search"])
        .edge("give_up", END)
        // Routes, not edges, into `next_candidate`: a node with several incoming edges waits for
        // all of them (an AND join), while it must run after whichever of these came first.
        .route("dedup", |_| "next_candidate", &["next_candidate"])
        .route(
            "next_candidate",
            |s| {
                if s.current.is_some() {
                    "fetch"
                } else if !s.paged_out {
                    "search"
                } else {
                    "finish"
                }
            },
            &["fetch", "search", "finish"],
        )
        .route("fetch", |s| if s.current.is_some() { "challenge_page" } else { "next_candidate" }, &["challenge_page", "next_candidate"])
        .route("challenge_page", |s| after_check(s, "filter"), &["give_up", "wait_human", "filter"])
        .route("filter", |s| if s.verdict.is_some() { "dismiss" } else { "fan" }, &["dismiss", "fan"])
        .parallel("fan", &["score", "insight"], "save_lead")
        .route("save_lead", |_| "next_candidate", &["next_candidate"])
        .route("dismiss", |_| "next_candidate", &["next_candidate"])
        .edge("finish", END)
}

/// `search`: the next page of the adapter, from the saved cursor. The cursor itself moves on in
/// `dedup`, so a page searched again after "Continue" is the same page. An adapter that meets a
/// login, captcha or block page in the browser ([`ChallengeFound`]) does not fail the source: the
/// challenge is raised and the `challenge` node sends the source to `wait_human`.
async fn search(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let source = sh.source(s.source_id)?;
    let adapter = sh.adapter(&source)?;
    let prefs = sh.store.preferences()?;
    let page_size = sh.deps.limits.page_size.max(1);
    let searched = adapter.search(&prefs, &SearchCtx { cursor: s.cursor.clone(), page_size }).await;
    let page = match searched {
        Ok(page) => page,
        Err(e) => {
            if let Some(found) = e.downcast_ref::<ChallengeFound>() {
                raise(&sh, &mut s, &source, found.kind, found.url.clone())?;
                return Ok(s);
            }
            return Err(e.context(format!("searching the {} source", source.kind)));
        }
    };
    s.page = page
        .candidates
        .into_iter()
        .take(page_size)
        .map(|c| Candidate { snippet: truncate_chars(&c.snippet, SNIPPET_CHARS).to_string(), ..c })
        .collect();
    s.page_next = page.next;
    s.scanned += 1;
    tracing::info!(run = %s.run, source_id = s.source_id, found = s.page.len(), "source page searched");
    Ok(s)
}

/// `challenge` and `challenge_page`: ask the hook; on a hit raise it. A challenge the search or the
/// fetch already raised (a page the browser rendered) is not asked again, nor counted twice.
async fn check(sh: Arc<Shared>, mut s: SourceState, stage: CheckStage) -> Result<SourceState> {
    if s.challenge.is_some() {
        return Ok(s);
    }
    let source = sh.source(s.source_id)?;
    let (url, text) = match (&s.current, stage) {
        (Some(c), CheckStage::Posting) => (c.url.clone(), truncate_chars(&s.jd, CHECK_TEXT_CHARS).to_string()),
        _ => (source.url.clone(), String::new()),
    };
    let found = (sh.deps.challenge)(ChallengeCheck { source: source.clone(), stage, url: url.clone(), text }).await?;
    match found {
        Some(kind) => raise(&sh, &mut s, &source, kind, url)?,
        // The page or the posting went through: the challenges are not "in a row" any more.
        None => s.strikes = 0,
    }
    Ok(s)
}

/// A login, captcha or block page was found at `url`: remember it in the state (the next node
/// pauses the source), mark the source and tell the user (unless this was the last strike, which
/// ends the source for the day instead).
fn raise(sh: &Shared, s: &mut SourceState, source: &Source, kind: ChallengeKind, url: String) -> Result<()> {
    s.strikes += 1;
    s.challenge = Some(Challenge { kind, source_id: source.id, url });
    tracing::warn!(run = %s.run, source_id = source.id, kind = ?kind, strikes = s.strikes, "source needs the user");
    if s.strikes < MAX_CHALLENGES_IN_A_ROW {
        sh.store.set_source_status(source.id, "needs_human")?;
        let name = if source.name.trim().is_empty() { source.kind.as_str() } else { source.name.trim() };
        (sh.deps.notify)("JobPilot needs you", &format!("{name}: {}", ask_of(kind)));
    }
    Ok(())
}

/// What the notification asks of the user.
fn ask_of(kind: ChallengeKind) -> &'static str {
    match kind {
        ChallengeKind::Login => "sign in to let the search go on",
        ChallengeKind::Captcha => "solve the captcha to let the search go on",
        ChallengeKind::Blocked => "the site blocks automated access; look at the page, then continue or skip it",
    }
}

/// `give_up`: the third challenge in a row. The app does not push on: the source rests for the
/// day and its thread ends.
async fn give_up(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let kind = s.challenge.take().map(|c| c.kind);
    sh.store.kv_set(&pause_key(s.source_id), &today())?;
    sh.store.set_source_run(s.source_id, "paused for today: repeated challenges")?;
    tracing::warn!(run = %s.run, source_id = s.source_id, kind = ?kind, "source paused for the day");
    s.stop = Some("paused for today after repeated challenges".into());
    s.current = None;
    s.jd.clear();
    Ok(s)
}

/// `dedup`: the page goes through: drop what is a lead (any state) or an application already, and
/// what is not an http(s) link; the rest joins the run's candidates. Moves the cursor on.
async fn dedup(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let known = Known::load(&sh.store)?;
    let mut fresh = 0u32;
    for c in std::mem::take(&mut s.page) {
        let Some(url) = canonical_url(&c.url) else { continue };
        let key = dedup_key(&c.company, &c.title);
        let seen_in_run = s.candidates.iter().any(|o| o.url == url || (!key.is_empty() && dedup_key(&o.company, &o.title) == key));
        if seen_in_run || known.contains(&sh.store, &url, &key)? {
            continue;
        }
        s.candidates.push(Candidate { url, ..c });
        fresh += 1;
    }
    s.cursor = s.page_next.take();
    s.pages += u32::from(fresh > 0);
    let limits = &sh.deps.limits;
    s.paged_out = s.cursor.is_none() || s.pages >= limits.pages_per_run || s.scanned >= limits.pages_per_run.saturating_mul(SCAN_FACTOR);
    tracing::info!(run = %s.run, source_id = s.source_id, new = fresh, "source page deduplicated");
    Ok(s)
}

/// `next_candidate`: take the next posting of the queue.
async fn next_candidate(_sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    s.jd.clear();
    s.verdict = None;
    s.scored = None;
    s.insight = None;
    s.current = s.candidates.get(s.taken).cloned();
    if s.current.is_some() {
        s.taken += 1;
    }
    Ok(s)
}

/// `fetch`: the posting's text, from the adapter when it has it, else through the fetch hook (whose
/// default keeps the per-host delay). A network failure fails the thread (Retry); a posting that
/// cannot be read, a bot check or login wall included, is skipped and stays unknown, so a later run
/// tries it again. Whatever leaves this node as an error has the posting's address taken out
/// ([`without_url`]): thread failures end up in notes, logs and the UI.
async fn fetch(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let Some(candidate) = s.current.clone() else { return Ok(s) };
    let url = candidate.url.clone();
    let source = sh.source(s.source_id)?;
    let adapter = sh.adapter(&source)?;
    let got = match adapter.posting(&candidate).await {
        Ok(Some(text)) => Ok(text),
        Ok(None) => (sh.deps.fetch)(FetchRequest { source: source.clone(), candidate }).await,
        Err(e) => Err(e),
    };
    // A login, captcha or block page is not a posting to skip: the posting stays the current one and
    // the source waits for the user (`challenge_page` -> `wait_human`), then fetches it again.
    let challenged = got.as_ref().err().and_then(|e| e.downcast_ref::<ChallengeFound>()).map(|f| (f.kind, f.url.clone()));
    if let Some((kind, challenge_url)) = challenged {
        s.jd.clear();
        raise(&sh, &mut s, &source, kind, challenge_url)?;
        return Ok(s);
    }
    match got.map_err(|e| without_url(e, &url)) {
        Ok(text) if text.trim().chars().count() >= MIN_POSTING_CHARS => {
            s.jd = truncate_chars(text.trim(), MAX_POSTING_CHARS).to_string();
        }
        Ok(_) => skip(&mut s, "too_short"),
        Err(e) if e.downcast_ref::<PageNeedsBrowser>().is_none() && classify(&format!("{e:#}")) == Problem::Network => {
            return Err(e.context("fetching a posting"))
        }
        Err(e) => skip(&mut s, skip_kind(&e)),
    }
    Ok(s)
}

/// A posting that cannot be read without a browser. Unlike [`NeedsBrowser`], whose text starts with
/// the page's address, this carries the reason only, so it may sit in an error chain.
#[derive(Debug)]
pub(super) struct PageNeedsBrowser {
    reason: String,
}

impl std::fmt::Display for PageNeedsBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the page needs a browser to render: {}", self.reason)
    }
}

impl std::error::Error for PageNeedsBrowser {}

const URL_MASK: &str = "<posting>";

/// `e` without the posting's address: [`NeedsBrowser`] becomes [`PageNeedsBrowser`], and any other
/// error that mentions `url` has it masked (the chain is flattened to text then).
pub(super) fn without_url(e: anyhow::Error, url: &str) -> anyhow::Error {
    if let Some(nb) = e.downcast_ref::<NeedsBrowser>() {
        let reason = nb.reason.replace(&nb.url, URL_MASK).replace(url, URL_MASK);
        return anyhow::Error::new(PageNeedsBrowser { reason });
    }
    let text = format!("{e:#}");
    if url.is_empty() || !text.contains(url) {
        return e;
    }
    anyhow::anyhow!("{}", text.replace(url, URL_MASK))
}

/// Why a posting could not be read, as a label (the error's text is not kept).
pub(super) fn skip_kind(e: &anyhow::Error) -> &'static str {
    if e.downcast_ref::<PageNeedsBrowser>().is_some() {
        return "needs_browser";
    }
    let text = format!("{e:#}");
    // The browser cannot be used right now (the extension is not connected, or may not open
    // Incognito tabs): nothing is wrong with the posting, and a later run reads it.
    let unavailable = matches!(e.downcast_ref::<BridgeError>(), Some(BridgeError::NotConnected | BridgeError::IncognitoNotAllowed));
    if unavailable || [NOT_CONNECTED, INCOGNITO_NOT_ALLOWED].iter().any(|t| text.contains(t)) {
        "browser_unavailable"
    } else if ["HTTP 404", "HTTP 410"].iter().any(|s| text.contains(s)) {
        "gone"
    } else if ["HTTP 401", "HTTP 403"].iter().any(|s| text.contains(s)) {
        "forbidden"
    } else {
        "unreadable"
    }
}

/// Count a posting left alone, by kind.
fn count_skip(s: &mut SourceState, kind: &'static str) {
    s.skipped += 1;
    *s.skip_kinds.entry(kind.to_string()).or_default() += 1;
}

fn skip(s: &mut SourceState, kind: &'static str) {
    tracing::warn!(run = %s.run, source_id = s.source_id, kind, "posting skipped");
    count_skip(s, kind);
    s.current = None;
    s.jd.clear();
}

/// `filter`: the hard filters, in code.
async fn filter(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let Some(candidate) = &s.current else { return Ok(s) };
    let prefs = sh.store.preferences()?;
    s.verdict = hard_filter(&prefs, candidate, &s.jd);
    Ok(s)
}

/// `score`: one model call; the total is computed in code.
async fn score_node(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let Some(candidate) = &s.current else { return Ok(s) };
    let prefs = sh.store.preferences()?;
    s.scored = Some(score(sh.deps.llm.as_ref(), &prefs, &sh.profile, candidate, &s.jd).await?);
    Ok(s)
}

/// `insight`: the job insight hook. Optional: a failure leaves the lead without insight.
async fn insight_node(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let Some(candidate) = &s.current else { return Ok(s) };
    let request = InsightRequest {
        url: candidate.url.clone(),
        company: candidate.company.clone(),
        role: candidate.title.clone(),
        jd_text: s.jd.clone(),
    };
    s.insight = match (sh.deps.insight)(request).await {
        Ok(json) => json.filter(|j| serde_json::from_str::<Value>(j).is_ok()),
        Err(_) => {
            tracing::warn!(run = %s.run, source_id = s.source_id, "insight failed, the lead is saved without it");
            None
        }
    };
    Ok(s)
}

/// `save_lead`: the lead, with its score and insight. When the model read that the posting hires
/// only outside `Preferences.work_from` ([`Eligible::No`](super::score::Eligible::No)), the lead
/// is stored dismissed with that reason (and its score), like one the hard filters dropped.
async fn save_lead(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let (score, score_json) = match &s.scored {
        Some(scored) => (scored.total, Some(scored.to_json()?)),
        None => (None, None),
    };
    let prefs = sh.store.preferences()?;
    let not_eligible = s.scored.as_ref().and_then(|scored| ineligible_reason(&prefs, scored));
    let insight_json = s.insight.take();
    let (state, reason) = match not_eligible {
        Some(reason) => (LeadState::Dismissed, Some(reason)),
        None => (LeadState::New, None),
    };
    match store_lead(&sh, &s, state, reason, score, score_json, insight_json)? {
        Stored::Saved if state == LeadState::Dismissed => s.dismissed += 1,
        Stored::Saved => s.saved += 1,
        Stored::AlreadyKnown => count_skip(&mut s, "duplicate"),
    }
    clear_current(&mut s);
    Ok(s)
}

/// `dismiss`: the hard filters rejected it; it is stored with the reason so it is not fetched again.
async fn dismiss(sh: Arc<Shared>, mut s: SourceState) -> Result<SourceState> {
    let reason = s.verdict.take();
    match store_lead(&sh, &s, LeadState::Dismissed, reason, None, None, None)? {
        Stored::Saved => s.dismissed += 1,
        Stored::AlreadyKnown => count_skip(&mut s, "duplicate"),
    }
    clear_current(&mut s);
    Ok(s)
}

fn clear_current(s: &mut SourceState) {
    s.current = None;
    s.jd.clear();
    s.scored = None;
    s.insight = None;
    s.verdict = None;
}

/// `finish`: the source ran to its end. A source that found nothing on a page pointing to a job
/// board (a careers page embedding Greenhouse) says so in its status, so the user can add that board.
async fn finish(sh: Arc<Shared>, s: SourceState) -> Result<SourceState> {
    let hints = if s.candidates.is_empty() { sh.adapter_of(s.source_id).map(|a| a.board_hints()).unwrap_or_default() } else { Vec::new() };
    let status = no_postings_status(&hints);
    if status.is_some() {
        tracing::info!(run = %s.run, source_id = s.source_id, boards = hints.len(), "source found no postings but points to a job board");
    }
    sh.store.set_source_run(s.source_id, status.as_deref().unwrap_or("ok"))?;
    tracing::info!(run = %s.run, source_id = s.source_id, saved = s.saved, dismissed = s.dismissed, skipped = s.skipped, pages = s.scanned, "source run finished");
    Ok(s)
}

enum Stored {
    Saved,
    /// Another source or run saved the posting meanwhile.
    AlreadyKnown,
}

/// Insert the current posting as a lead and write its text to `<data_dir>/leads/<id>/jd.md`. A
/// node that ran before (and died before its checkpoint) finds the lead and only repeats the file.
fn store_lead(
    sh: &Shared,
    s: &SourceState,
    state: LeadState,
    dismiss_reason: Option<String>,
    score: Option<f64>,
    score_json: Option<String>,
    insight_json: Option<String>,
) -> Result<Stored> {
    let Some(c) = &s.current else { return Ok(Stored::AlreadyKnown) };
    let source = sh.source(s.source_id)?;
    let company = if c.company.trim().is_empty() { source.name.trim().to_string() } else { c.company.trim().to_string() };
    let key = dedup_key(&company, &c.title);
    let lead = {
        let _one_at_a_time = sh.save.lock().unwrap_or_else(|e| e.into_inner());
        match sh.store.find_lead(&c.url, "")? {
            Some(mine) if mine.source_id == Some(source.id) => mine,
            Some(_) => return Ok(Stored::AlreadyKnown),
            None if sh.store.lead_exists(&c.url, &key)? || Known::load(&sh.store)?.has_application(&c.url, &key) => {
                return Ok(Stored::AlreadyKnown)
            }
            None => {
                let new = NewLead {
                    source_id: Some(source.id),
                    url: c.url.clone(),
                    dedup_key: key,
                    company: company.clone(),
                    role: c.title.clone(),
                    location: c.location.clone(),
                    jd_path: None,
                    score,
                    score_json,
                    insight_json,
                    state,
                    dismiss_reason,
                };
                sh.store.add_lead(&new)?
            }
        }
    };
    attach_jd(&sh.store, &sh.paths, lead.id, &company, c, &s.jd)?;
    tracing::info!(run = %s.run, source_id = source.id, lead_id = lead.id, state = state.as_str(), "lead stored");
    Ok(Stored::Saved)
}

/// `<data_dir>/leads/<id>/`.
pub fn lead_dir(paths: &Paths, lead_id: i64) -> std::path::PathBuf {
    paths.data_dir.join("leads").join(lead_id.to_string())
}

/// Write the posting text of lead `lead_id` to its `jd.md` and record where it is. Repeating it
/// only rewrites the file, and the file is replaced whole (written beside it, then renamed), so
/// two writers at once (the same job added twice) never leave it cut short.
/// Shared with the leads the user adds by hand (`discover::manual`).
pub(super) fn attach_jd(store: &Store, paths: &Paths, lead_id: i64, company: &str, c: &Candidate, text: &str) -> Result<std::path::PathBuf> {
    let path = write_jd(paths, lead_id, company, c, text)?;
    store.set_lead_jd_path(lead_id, &path)?;
    Ok(path)
}

/// `jd.md` in the layout of an application's (`# company - role`, `Source:`, a blank line, the
/// text), so a promoted lead can use it as it is.
fn write_jd(paths: &Paths, lead_id: i64, company: &str, c: &Candidate, text: &str) -> Result<std::path::PathBuf> {
    let dir = lead_dir(paths, lead_id);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("jd.md");
    let body = format!("# {company} \u{2014} {}\nSource: {}\n\n{}\n", c.title, c.url, text.trim());
    write_atomic(&path, &body)?;
    Ok(path)
}

/// Replace `path` with `body` in one step: a temporary file of its own in the same directory (the
/// same file system, so the rename is atomic) is written first.
fn write_atomic(path: &std::path::Path, body: &str) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!("md.{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::Error::new(e).context(format!("replacing {}", path.display()))
    })
}

// ---------------------------------------------------------------- pausing a source for a day

fn pause_key(source_id: i64) -> String {
    format!("discover.paused.{source_id}")
}

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn paused_today(store: &Store, source_id: i64) -> Result<bool> {
    Ok(store.kv_get(&pause_key(source_id))?.as_deref() == Some(today().as_str()))
}

// ---------------------------------------------------------------- the discovery graph

fn discovery_graph(sh: &Arc<Shared>, source: GraphDef<SourceState>) -> GraphDef<DiscoveryState> {
    let plan_sh = sh.clone();
    let run_sh = sh.clone();
    let summary_sh = sh.clone();
    GraphDef::new(DISCOVERY_GRAPH)
        .node("plan", move |s| plan(plan_sh.clone(), s))
        .node("sources", move |s| run_sources(run_sh.clone(), source.clone(), s))
        .node("summary", move |s| summarize(summary_sh.clone(), s))
        .node_timeout("sources", SOURCES_TIMEOUT)
        .edge(START, "plan")
        .edge("plan", "sources")
        .edge("sources", "summary")
        .edge("summary", END)
}

/// `plan`: the enabled job sources, minus those resting for the day and those still waiting for
/// the user from an earlier run.
async fn plan(sh: Arc<Shared>, mut s: DiscoveryState) -> Result<DiscoveryState> {
    let waiting: HashSet<i64> = sh
        .graphs
        .waiting()?
        .iter()
        .filter(|w| w.graph == SOURCE_GRAPH)
        .filter_map(|w| parse_source_thread(&w.thread).map(|(_, id)| id))
        .collect();
    s.sources.clear();
    s.results.clear();
    for source in sh.store.list_enabled_sources(SourcePurpose::Jobs)? {
        let thread = source_thread(&s.run, source.id);
        let not_run = if paused_today(&sh.store, source.id)? {
            Some("paused for today after repeated challenges")
        } else if waiting.contains(&source.id) {
            Some("still waiting for you from an earlier run")
        } else {
            None
        };
        match not_run {
            Some(why) => s.results.push(SourceResult::of(&thread, source.id, SourceStatus::NotRun, None, why.into())),
            None => s.sources.push(source.id),
        }
    }
    tracing::info!(run = %s.run, sources = s.sources.len(), not_run = s.results.len(), "discovery planned");
    Ok(s)
}

/// `sources`: one `source_run` thread per source, `sources_in_parallel` at a time.
async fn run_sources(sh: Arc<Shared>, source: GraphDef<SourceState>, mut s: DiscoveryState) -> Result<DiscoveryState> {
    let jobs = s.sources.iter().map(|id| (source_thread(&s.run, *id), SourceState::new(&s.run, *id))).collect();
    for (thread, outcome) in sh.graphs.run_each(&source, jobs, sh.deps.limits.sources_in_parallel).await {
        let Some((_, source_id)) = parse_source_thread(&thread) else { continue };
        let result = source_result(&sh.store, &thread, source_id, outcome);
        s.results.push(result);
    }
    Ok(s)
}

/// How a source thread ended, recorded on the source row (`failed: ...`) and as a [`SourceResult`].
pub(crate) fn source_result(store: &Store, thread: &str, source_id: i64, outcome: Result<Outcome<SourceState>>) -> SourceResult {
    // A thread that did not finish has no final state; what it stored so far is in the leads.
    let so_far = |status, note: String| {
        let mut result = SourceResult::of(thread, source_id, status, None, note);
        if let Some((saved, dismissed)) = parse_source_thread(thread).and_then(|(run, _)| lead_counts(store, source_id, &run)) {
            (result.saved, result.dismissed) = (saved, dismissed);
        }
        result
    };
    let failed = |message: String| {
        // The graph runtime masks secrets already; the note is shown and stored, so mask again.
        let note = one_line(&redact(&message), 160);
        if let Err(e) = store.set_source_run(source_id, &format!("failed: {note}")) {
            tracing::warn!(source_id, error = %e, "recording a failed source failed");
        }
        tracing::warn!(thread, source_id, "source run failed");
        so_far(SourceStatus::Failed, note)
    };
    match outcome {
        Ok(Outcome::Done(state)) => {
            SourceResult::of(thread, source_id, SourceStatus::Done, Some(&state), state.stop.clone().unwrap_or_default())
        }
        Ok(Outcome::Waiting(reason)) => so_far(SourceStatus::Waiting, wait_label(&reason).into()),
        Ok(Outcome::Failed(f)) => failed(f.message),
        Ok(Outcome::Cancelled) => SourceResult::of(thread, source_id, SourceStatus::Cancelled, None, String::new()),
        Err(e) => failed(format!("{e:#}")),
    }
}

/// Leads of `source_id` stored since the run started (its id is the start time in milliseconds):
/// `(new, dismissed)`.
fn lead_counts(store: &Store, source_id: i64, run: &str) -> Option<(u32, u32)> {
    let since = chrono::DateTime::from_timestamp_millis(run.parse().ok()?)?;
    let count = |state| -> Option<u32> {
        let leads = store.list_leads(state).ok()?;
        Some(leads.iter().filter(|l| l.source_id == Some(source_id) && l.found_at >= since).count() as u32)
    };
    Some((count(LeadState::New)?, count(LeadState::Dismissed)?))
}

/// `summary`: the counts, kept as kv `discover.last_run`.
async fn summarize(sh: Arc<Shared>, mut s: DiscoveryState) -> Result<DiscoveryState> {
    let finished = chrono::Utc::now().to_rfc3339();
    let summary = RunSummary::of(&s.run, &s.started_at, &finished, s.results.clone());
    sh.store.kv_set(LAST_RUN_KEY, &serde_json::to_string(&summary).context("writing the run summary")?)?;
    tracing::info!(
        run = %s.run,
        saved = summary.saved,
        dismissed = summary.dismissed,
        waiting = summary.waiting,
        failed = summary.failed,
        "discovery run finished"
    );
    s.summary = Some(summary);
    Ok(s)
}

/// A new run id: the start time in milliseconds, so ids sort by time.
pub(crate) fn new_run_id() -> String {
    chrono::Utc::now().timestamp_millis().to_string()
}

pub(crate) fn new_discovery(run: &str) -> DiscoveryState {
    DiscoveryState { run: run.to_string(), started_at: chrono::Utc::now().to_rfc3339(), ..DiscoveryState::default() }
}
