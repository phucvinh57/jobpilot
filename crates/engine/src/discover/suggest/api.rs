//! The `Core` methods of "Suggest sources".

use anyhow::{anyhow, bail, Context, Result};

use super::graph::{graph, SuggestState, SUGGEST_GRAPH, SUGGEST_THREAD};
use super::queries::{build_queries_for, SuggestTerms};
use super::SuggestDeps;
use crate::discover::handoff;
use crate::graph::{Outcome, ThreadStatus, WaitReason, Waiting};
use crate::store::{Source, SourceSuggestion, SuggestionState};
use crate::Core;

/// The kv key of the terms the user last picked for "Suggest sources" (JSON of [`SuggestTerms`]).
pub const SUGGEST_TERMS_KEY: &str = "discover.suggest.terms";

/// What the "Suggest sources" dialog shows: the values the user can pick, and the ones picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuggestOptions {
    /// What the preferences (or the profile's roles) offer ([`SuggestTerms::available`]).
    pub available: SuggestTerms,
    /// The remembered pick within `available`, or [`SuggestTerms::default_pick`] without one.
    pub picked: SuggestTerms,
}

/// How a suggestion search ended this time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuggestOutcome {
    /// Every query was searched and the candidates checked: `saved` suggestions are stored as new
    /// (list them with [`Core::source_suggestions`]).
    Done { saved: usize },
    /// The search engine showed a consent page, captcha or login: the user deals with it
    /// ([`Core::suggest_open_browser`]), then [`Core::suggest_resume`].
    Waiting { thread: String, reason: WaitReason },
}

impl Core {
    /// "Suggest sources": search the web for job boards and careers pages that fit the wanted roles,
    /// check them, and store the ones with jobs as suggestions. A search paused at a challenge is
    /// continued; a failed one starts over. Long work (minutes): run it on the global runtime and
    /// follow `core.graphs.subscribe()` (graph [`SUGGEST_GRAPH`]). `terms`: the values the user
    /// picked (kept only within what the preferences offer, and remembered for next time); `None`:
    /// the default pick. A paused search continues with the terms it started with. `Err`
    /// (classified by `health::classify`) when there is nothing to search for (no roles, no
    /// profile, an empty pick), the browser cannot be used or the network is down.
    pub async fn suggest_sources(&self, terms: Option<SuggestTerms>) -> Result<SuggestOutcome> {
        self.suggest_sources_with(SuggestDeps::live(self)?, terms).await
    }

    /// [`suggest_sources`](Self::suggest_sources) with explicit [`SuggestDeps`].
    pub async fn suggest_sources_with(&self, deps: SuggestDeps, terms: Option<SuggestTerms>) -> Result<SuggestOutcome> {
        match self.graphs.status(SUGGEST_THREAD)?.map(|t| t.status) {
            Some(ThreadStatus::Running) => bail!("a search for sources is running already"),
            Some(ThreadStatus::Waiting(_)) => return self.suggest_resume_with(deps).await,
            Some(ThreadStatus::Failed(_)) => self.suggest_cancel()?,
            Some(ThreadStatus::Done) | None => {}
        }
        let terms = match terms {
            Some(terms) => {
                // An empty pick fails here, before it is remembered (the dialog would open empty).
                build_queries_for(&terms.within(&self.suggest_available()?))?;
                Some(self.remember_suggest_terms(&terms)?)
            }
            None => None,
        };
        tracing::info!(picked = terms.is_some(), "source suggestion search started");
        let start = SuggestState { terms, ..SuggestState::default() };
        let outcome = self.graphs.run(&graph(self, deps), SUGGEST_THREAD, start).await?;
        outcome_of(outcome)
    }

    /// What the "Suggest sources" dialog offers and has picked (see [`SuggestOptions`]). A
    /// remembered pick that cannot be read counts as none.
    pub fn suggest_options(&self) -> Result<SuggestOptions> {
        let available = self.suggest_available()?;
        let picked = match self.remembered_suggest_terms()? {
            Some(terms) => terms.within(&available),
            None => SuggestTerms::default_pick(&available),
        };
        Ok(SuggestOptions { available, picked })
    }

    /// Remember the user's pick for the next "Suggest sources", without the values the preferences
    /// no longer offer. Returns what was remembered.
    pub fn remember_suggest_terms(&self, terms: &SuggestTerms) -> Result<SuggestTerms> {
        let picked = terms.within(&self.suggest_available()?);
        let json = serde_json::to_string(&picked).context("writing the picked search terms")?;
        self.store.kv_set(SUGGEST_TERMS_KEY, &json)?;
        tracing::info!(
            roles = picked.roles.len(),
            domains = picked.domains.len(),
            skills = picked.skills.len(),
            places = picked.places.len(),
            remote = picked.remote,
            "source suggestion terms remembered"
        );
        Ok(picked)
    }

    /// What the preferences (or, without roles, the profile) offer to search for.
    fn suggest_available(&self) -> Result<SuggestTerms> {
        let prefs = self.store.preferences()?;
        let cv = self.profile()?.map(|p| p.cv);
        Ok(SuggestTerms::available(&prefs, cv.as_ref()))
    }

    /// The remembered pick, if there is one that can be read.
    fn remembered_suggest_terms(&self) -> Result<Option<SuggestTerms>> {
        let Some(json) = self.store.kv_get(SUGGEST_TERMS_KEY)? else { return Ok(None) };
        match serde_json::from_str(&json) {
            Ok(terms) => Ok(Some(terms)),
            Err(e) => {
                tracing::warn!(error = %e, "the remembered source suggestion terms could not be read, ignored");
                Ok(None)
            }
        }
    }

    /// "Continue" after the user dealt with the page the search waits at (and "Retry" after a
    /// failure): the tab opened for them closes and the search goes on with the same query.
    pub async fn suggest_resume(&self) -> Result<SuggestOutcome> {
        self.suggest_resume_with(SuggestDeps::live(self)?).await
    }

    /// [`suggest_resume`](Self::suggest_resume) with explicit [`SuggestDeps`].
    pub async fn suggest_resume_with(&self, deps: SuggestDeps) -> Result<SuggestOutcome> {
        match self.graphs.status(SUGGEST_THREAD)?.map(|t| t.status) {
            Some(ThreadStatus::Waiting(_) | ThreadStatus::Failed(_)) => {}
            Some(ThreadStatus::Running) => bail!("a search for sources is running already"),
            Some(ThreadStatus::Done) | None => bail!("no search for sources is waiting"),
        }
        handoff::close_window(&self.paths, SUGGEST_THREAD).await;
        tracing::info!("source suggestion search resumed");
        let outcome = self.graphs.resume(&graph(self, deps), SUGGEST_THREAD, |s| s.challenge = None).await?;
        outcome_of(outcome)
    }

    /// Stop the search (running, waiting or failed) and forget it; the suggestions stored so far
    /// stay. Nothing to stop is fine.
    pub fn suggest_cancel(&self) -> Result<()> {
        handoff::discard_window(&self.paths, SUGGEST_THREAD);
        if self.graphs.status(SUGGEST_THREAD)?.is_some() {
            self.graphs.cancel(SUGGEST_THREAD)?;
            tracing::info!("source suggestion search cancelled");
        }
        Ok(())
    }

    /// A search is running now (the screen keeps "Suggest sources" disabled when it is shown
    /// again mid-search).
    pub fn suggest_running(&self) -> Result<bool> {
        Ok(matches!(self.graphs.status(SUGGEST_THREAD)?.map(|t| t.status), Some(ThreadStatus::Running)))
    }

    /// The search paused at a consent page, captcha or login, if there is one (for the "Open
    /// browser" banner). Survives a restart.
    pub fn suggest_waiting(&self) -> Result<Option<Waiting>> {
        Ok(self.graphs.waiting()?.into_iter().find(|w| w.graph == SUGGEST_GRAPH))
    }

    /// "Open browser" for a paused search: a focused tab of the user's Chrome, in the JobPilot
    /// Incognito window, on the page the search engine showed. The user deals with it; the app
    /// does nothing in it. [`suggest_resume`](Self::suggest_resume) closes it.
    pub async fn suggest_open_browser(&self) -> Result<()> {
        handoff::open_waiting_page(self, SUGGEST_THREAD, "search", false, |_| Ok(None)).await
    }

    /// The suggestions waiting for the user, the most matching postings first.
    pub fn source_suggestions(&self) -> Result<Vec<SourceSuggestion>> {
        self.store.list_suggestions(SuggestionState::New)
    }

    /// Dismiss a suggestion: no later search suggests it again. One that was added stays added.
    pub fn dismiss_source_suggestion(&self, id: i64) -> Result<()> {
        let suggestion = self.store.get_suggestion(id)?.ok_or_else(|| anyhow!("suggestion {id} not found"))?;
        if suggestion.state == SuggestionState::New {
            self.store.set_suggestion_state(id, SuggestionState::Dismissed)?;
            tracing::info!(suggestion_id = id, "source suggestion dismissed");
        }
        Ok(())
    }

    /// "Add" on a suggestion: store it as a job source and mark it added. The search already read
    /// it ([`Core::check_job_source`]), so only the address check runs again (no second read);
    /// an added or dismissed suggestion is refused.
    pub fn add_suggested_source(&self, id: i64) -> Result<Source> {
        let suggestion = self.store.get_suggestion(id)?.ok_or_else(|| anyhow!("suggestion {id} no longer exists"))?;
        if suggestion.state != SuggestionState::New {
            bail!("this suggestion was already {}", suggestion.state.as_str());
        }
        let source = self.add_job_source(&suggestion.kind, &suggestion.name, &suggestion.input, None)?;
        self.mark_source_suggestion_added(id)?;
        Ok(source)
    }

    /// Record that a suggestion was added as a job source (after `add_job_source` succeeded).
    pub fn mark_source_suggestion_added(&self, id: i64) -> Result<()> {
        self.store.set_suggestion_state(id, SuggestionState::Added)?;
        tracing::info!(suggestion_id = id, "source suggestion added");
        Ok(())
    }
}

/// A finished run or resume as the caller sees it: a failure is the `Err` whose text
/// `health::classify` reads (no profile, the browser, the network).
fn outcome_of(outcome: Outcome<SuggestState>) -> Result<SuggestOutcome> {
    match outcome {
        Outcome::Done(state) => Ok(SuggestOutcome::Done { saved: state.saved }),
        Outcome::Waiting(reason) => Ok(SuggestOutcome::Waiting { thread: SUGGEST_THREAD.to_string(), reason }),
        Outcome::Failed(failure) => Err(anyhow!("{}", failure.message)),
        Outcome::Cancelled => bail!("the search for sources was cancelled"),
    }
}
