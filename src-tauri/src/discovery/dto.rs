//! Read models of this context (serde, camelCase). Core types stay free of serde and Tauri.
//!
//! Nothing secret travels here: leads, sources, run summaries and session names only. Posting and
//! brief text is data for the web app to show as text.

use crate::engine::discover::promote::{Dismissed, FailedPrepare};
use crate::engine::discover::queue::{criteria_rows, QueueItem, QueueSnapshot};
use crate::engine::discover::{RunSummary, WaitingSource};
use crate::engine::graph::WaitReason;
use crate::engine::health;
use crate::engine::pipeline::JdInput;
use crate::engine::store::{Application, Lead, Session, Source};
use serde::{Deserialize, Serialize};

use super::rules::host_of;
use crate::shared::insight::is_web_link;
use crate::shared::{CommandError, InsightDto, ProblemDto};

// ------------------------------------------------------------------ the link box

/// What the user put in the link box (or dropped on the window), classified by
/// [`rules::parse_input`](super::rules::parse_input).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum LeadInput {
    /// A posting link (http or https).
    Url(String),
    /// A path to a `.md`, `.txt`, `.html`, `.htm` or `.pdf` file.
    File(String),
    /// The posting's text.
    Text(String),
}

impl LeadInput {
    /// The input as the core reads it.
    pub fn into_jd(self) -> JdInput {
        match self {
            LeadInput::Url(url) => JdInput::Url(url),
            LeadInput::File(path) => JdInput::File(path.into()),
            LeadInput::Text(text) => JdInput::Text(text),
        }
    }
}

/// What became of a posting that was added: the lead, and where it stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedLeadDto {
    pub lead_id: i64,
    /// `new` / `dismissed` / `promoted`.
    pub state: &'static str,
    /// The application of a lead whose CV was prepared before.
    pub application_id: Option<i64>,
}

impl From<&Lead> for AddedLeadDto {
    fn from(lead: &Lead) -> Self {
        Self {
            lead_id: lead.id,
            state: lead.state.as_str(),
            application_id: lead.app_id,
        }
    }
}

// ------------------------------------------------------------------ the queue

/// One criterion of "why it fits": the weight it counted with, the score and the reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CriterionDto {
    /// `skills` / `domain` / `location` / `salary` / `seniority`.
    pub criterion: &'static str,
    pub weight: u32,
    /// `false` when neither the posting nor the preferences say anything about it.
    pub known: bool,
    pub score: u32,
    pub reason: String,
}

/// One line of "why this score" under a lead's name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonDto {
    pub criterion: &'static str,
    pub score: u32,
    pub text: String,
}

/// A lead as the queue shows it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadDto {
    pub id: i64,
    /// The source that found it; `None` for a job the user added (or a deleted source).
    pub source_id: Option<i64>,
    /// The posting's address. Not a web link for a job added from a file or pasted text.
    pub url: String,
    pub company: String,
    pub role: String,
    pub location: String,
    /// 0 to 100; `None` when it was never scored.
    pub score: Option<f64>,
    /// When it was found (RFC 3339).
    pub found_at: String,
    /// The reasons that count most for the score (at most two).
    pub reasons: Vec<ReasonDto>,
    /// The five criteria in a fixed order; `None` when the lead has no readable score breakdown.
    pub criteria: Option<Vec<CriterionDto>>,
    /// The company brief; `None` when the lead has none.
    pub insight: Option<InsightDto>,
}

impl From<&QueueItem> for LeadDto {
    fn from(item: &QueueItem) -> Self {
        let lead = &item.lead;
        Self {
            id: lead.id,
            source_id: lead.source_id,
            url: lead.url.clone(),
            company: lead.company.clone(),
            role: lead.role.clone(),
            location: lead.location.clone(),
            score: lead.score,
            found_at: lead.found_at.to_rfc3339(),
            reasons: item
                .reasons(2)
                .into_iter()
                .map(|r| ReasonDto {
                    criterion: r.criterion,
                    score: r.score,
                    text: r.text,
                })
                .collect(),
            criteria: item.scored.as_ref().map(|scored| {
                criteria_rows(scored)
                    .into_iter()
                    .map(|r| CriterionDto {
                        criterion: r.criterion,
                        weight: r.weight,
                        known: r.detail.known,
                        score: r.detail.score,
                        reason: r.detail.reason,
                    })
                    .collect()
            }),
            insight: item.insight.as_ref().map(InsightDto::from),
        }
    }
}

/// A job source and how its last run ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub id: i64,
    /// `greenhouse` / `lever` / `ashby` / `smartrecruiters` / `recruitee` / `personio` / `workday` / `hn_hiring` / `yc` / `career_page`.
    pub kind: String,
    /// The name the user (or the adapter) gave; may be blank.
    pub name: String,
    /// The host of its address (never the whole address: it may carry a token).
    pub host: String,
    pub enabled: bool,
    /// The managed browser session it is read in.
    pub session_id: Option<i64>,
    /// RFC 3339.
    pub last_run_at: Option<String>,
    /// Free text set by the runner: `ok`, `needs_human`, `failed: ...`.
    pub last_status: Option<String>,
}

impl From<&Source> for SourceDto {
    fn from(s: &Source) -> Self {
        Self {
            id: s.id,
            kind: s.kind.clone(),
            name: s.name.clone(),
            host: host_of(&s.url),
            enabled: s.enabled,
            session_id: s.session_id,
            last_run_at: s.last_run_at.map(|t| t.to_rfc3339()),
            last_status: s.last_status.clone(),
        }
    }
}

/// A source paused at a login, captcha or block page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingDto {
    /// What `discovery_open_browser`, `discovery_resume` and `discovery_skip` take.
    pub thread: String,
    pub source_id: i64,
    pub source_name: String,
    /// `login` / `captcha` / `blocked` / `other`.
    pub reason: &'static str,
    /// The page the user has to deal with (an http(s) link only).
    pub url: Option<String>,
}

impl From<&WaitingSource> for WaitingDto {
    fn from(w: &WaitingSource) -> Self {
        let (reason, url) = match &w.reason {
            WaitReason::Login { url, .. } => ("login", Some(url)),
            WaitReason::Captcha { url, .. } => ("captcha", Some(url)),
            WaitReason::Blocked { url, .. } => ("blocked", Some(url)),
            _ => ("other", None),
        };
        Self {
            thread: w.thread.clone(),
            source_id: w.source_id,
            source_name: w.source_name.clone(),
            reason,
            url: url.filter(|u| is_web_link(u)).cloned(),
        }
    }
}

/// A "Prepare CV" that did not finish (retry it, open its application, or forget it).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedPrepareDto {
    pub lead_id: i64,
    pub company: String,
    pub role: String,
    /// The error text, secrets masked.
    pub message: String,
    pub problem: ProblemDto,
    /// The application that was already created for the lead.
    pub application_id: Option<i64>,
}

impl From<&FailedPrepare> for FailedPrepareDto {
    fn from(f: &FailedPrepare) -> Self {
        Self {
            lead_id: f.lead.id,
            company: f.lead.company.clone(),
            role: f.lead.role.clone(),
            message: f.failure.message.clone(),
            problem: health::classify(&f.failure.message).into(),
            application_id: f.lead.app_id,
        }
    }
}

/// What a search found (the last one, or the one that just ended).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryDto {
    /// RFC 3339; blank when the summary has no end time.
    pub finished_at: String,
    /// New leads.
    pub saved: u32,
    /// Sources waiting for the user.
    pub waiting: u32,
    pub failed: u32,
}

impl From<&RunSummary> for RunSummaryDto {
    fn from(s: &RunSummary) -> Self {
        Self {
            finished_at: s.finished_at.clone(),
            saved: s.saved,
            waiting: s.waiting,
            failed: s.failed,
        }
    }
}

/// Everything Today shows, read in one go.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueDto {
    /// Leads waiting for the user, best match first.
    pub items: Vec<LeadDto>,
    pub failed: Vec<FailedPrepareDto>,
    pub sources: Vec<SourceDto>,
    pub waiting: Vec<WaitingDto>,
    pub last_run: Option<RunSummaryDto>,
    /// A search (or a source being continued) is running right now, started from here or by the
    /// daily timer.
    pub searching: bool,
}

impl QueueDto {
    pub fn new(snapshot: &QueueSnapshot, searching: bool) -> Self {
        Self {
            items: snapshot.items.iter().map(LeadDto::from).collect(),
            failed: snapshot.failed.iter().map(FailedPrepareDto::from).collect(),
            sources: snapshot.sources.iter().map(SourceDto::from).collect(),
            waiting: snapshot.waiting.iter().map(WaitingDto::from).collect(),
            last_run: snapshot.last_run.as_ref().map(RunSummaryDto::from),
            searching,
        }
    }
}

// ------------------------------------------------------------------ actions

/// What "Dismiss" did (the web app words it).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissedDto {
    /// Other leads of the company dismissed with it.
    pub also: usize,
    /// Other leads of the company kept because their CV is queued or being written.
    pub kept: usize,
    /// "Never this company" was asked for but could not be saved (why).
    pub not_excluded: Option<String>,
}

impl From<Dismissed> for DismissedDto {
    fn from(d: Dismissed) -> Self {
        Self {
            also: d.also,
            kept: d.kept,
            not_excluded: d.not_excluded,
        }
    }
}

/// How one lead's "Prepare CV" ended; also the payload of the `discovery:prepared` event.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareOutcomeDto {
    pub lead_id: i64,
    /// The application with its CV; `None` when it failed.
    pub application_id: Option<i64>,
    /// `Company — Role` of the application.
    pub label: Option<String>,
    pub error: Option<CommandError>,
}

impl PrepareOutcomeDto {
    pub fn new(lead_id: i64, result: anyhow::Result<Application>) -> Self {
        match result {
            Ok(app) => Self {
                lead_id,
                application_id: Some(app.id),
                label: Some(format!("{} \u{2014} {}", app.company, app.role)),
                error: None,
            },
            Err(e) => Self {
                lead_id,
                application_id: None,
                label: None,
                error: Some(CommandError::from(e)),
            },
        }
    }
}

/// How a search ended, for the web app to show under the header; the payload of the
/// `discovery:search-finished` event (the daily search) and the result of `discovery_search_now`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFinishedDto {
    pub summary: Option<RunSummaryDto>,
    pub error: Option<CommandError>,
}

impl SearchFinishedDto {
    pub fn new(result: anyhow::Result<RunSummary>) -> Self {
        match result {
            Ok(summary) => Self {
                summary: Some(RunSummaryDto::from(&summary)),
                error: None,
            },
            Err(e) => Self {
                summary: None,
                error: Some(CommandError::from(e)),
            },
        }
    }
}

// ------------------------------------------------------------------ Settings > Job sources

/// A kind of job source the user can add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceKindDto {
    pub kind: &'static str,
    pub label: &'static str,
    /// What to type in the address field.
    pub hint: &'static str,
}

/// A managed browser session a source can be read in (name only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOptionDto {
    pub id: i64,
    pub name: String,
}

impl From<&Session> for SessionOptionDto {
    fn from(s: &Session) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
        }
    }
}

/// Everything Settings > Job sources shows.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSourcesDto {
    pub sources: Vec<SourceDto>,
    /// The sources paused for the user right now.
    pub waiting: Vec<i64>,
    pub sessions: Vec<SessionOptionDto>,
    /// "Search daily while JobPilot is open".
    pub daily: bool,
    pub kinds: Vec<SourceKindDto>,
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use crate::engine::discover::queue::QueueItem;
    use crate::engine::discover::score::{Criteria, Criterion, Scored};
    use crate::engine::discover::WaitingSource;
    use crate::engine::graph::WaitReason;
    use crate::engine::prefs::Weights;
    use crate::engine::store::{Lead, LeadState, Source, SourcePurpose};

    use super::{
        AddedLeadDto, LeadDto, LeadInput, PrepareOutcomeDto, SearchFinishedDto, SourceDto,
        WaitingDto,
    };

    fn lead() -> Lead {
        Lead {
            id: 7,
            source_id: Some(2),
            url: "https://jobs.test/7".into(),
            dedup_key: "k".into(),
            company: "Acme".into(),
            role: "Backend Engineer".into(),
            location: "Remote".into(),
            jd_path: None,
            score: Some(78.0),
            score_json: None,
            insight_json: None,
            state: LeadState::New,
            dismiss_reason: None,
            app_id: None,
            found_at: Utc.with_ymd_and_hms(2026, 11, 14, 9, 30, 0).unwrap(),
        }
    }

    fn crit(score: u32, known: bool, reason: &str) -> Criterion {
        Criterion {
            score,
            known,
            reason: reason.into(),
        }
    }

    #[test]
    fn a_lead_carries_its_score_breakdown_reasons_and_brief() {
        let scored = Scored {
            total: Some(78.0),
            criteria: Criteria {
                skills: crit(90, true, "Rust and Postgres match"),
                domain: crit(40, true, "payments, not your domain"),
                location: crit(100, true, "remote"),
                salary: crit(0, false, ""),
                seniority: crit(70, true, ""),
            },
            weights: Weights::default(),
        };
        let mut l = lead();
        l.score_json = Some(scored.to_json().unwrap());
        l.insight_json =
            Some(r#"{"domain":"Payments","skills":{"required":["Rust"],"preferred":[]}}"#.into());
        let dto = LeadDto::from(&QueueItem::new(l));
        assert_eq!((dto.id, dto.source_id, dto.score), (7, Some(2), Some(78.0)));
        assert_eq!(dto.found_at, "2026-11-14T09:30:00+00:00");
        assert_eq!(
            dto.reasons.iter().map(|r| r.criterion).collect::<Vec<_>>(),
            ["skills", "location"]
        );
        let rows = dto.criteria.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.criterion).collect::<Vec<_>>(),
            ["skills", "domain", "location", "salary", "seniority"]
        );
        assert!(!rows[3].known, "a salary nobody stated is not judged");
        let insight = dto.insight.unwrap();
        assert_eq!(
            (insight.domain.as_str(), insight.required_skills.as_slice()),
            ("Payments", ["Rust".to_string()].as_slice())
        );
    }

    #[test]
    fn a_lead_without_a_readable_score_has_no_breakdown() {
        let mut l = lead();
        l.score_json = Some("not json".into());
        let dto = LeadDto::from(&QueueItem::new(l));
        assert!(dto.criteria.is_none() && dto.reasons.is_empty() && dto.insight.is_none());
    }

    #[test]
    fn a_waiting_source_names_its_page_only_when_it_is_a_web_link() {
        let waiting = |reason| WaitingSource {
            thread: "discover:1:3".into(),
            run: "1".into(),
            source_id: 3,
            source_name: "Acme careers".into(),
            reason,
            since: Utc.with_ymd_and_hms(2026, 11, 14, 9, 0, 0).unwrap(),
        };
        let login = WaitingDto::from(&waiting(WaitReason::Login {
            source_id: 3,
            url: "https://acme.test/login".into(),
        }));
        assert_eq!(
            (login.reason, login.url.as_deref()),
            ("login", Some("https://acme.test/login"))
        );
        let captcha = WaitingDto::from(&waiting(WaitReason::Captcha {
            source_id: 3,
            url: "https://acme.test/c".into(),
        }));
        assert_eq!(captcha.reason, "captcha");
        let blocked = WaitingDto::from(&waiting(WaitReason::Blocked {
            source_id: 3,
            url: "javascript:alert(1)".into(),
        }));
        assert_eq!(
            (blocked.reason, blocked.url),
            ("blocked", None),
            "only http(s) pages are offered"
        );
        let other = WaitingDto::from(&waiting(WaitReason::Unknown));
        assert_eq!((other.reason, other.url), ("other", None));
    }

    #[test]
    fn a_source_keeps_its_last_run() {
        let source = Source {
            id: 4,
            kind: "greenhouse".into(),
            name: "Acme".into(),
            url: "https://boards.greenhouse.io/acme".into(),
            purpose: SourcePurpose::Jobs,
            enabled: true,
            session_id: Some(9),
            last_run_at: Some(Utc.with_ymd_and_hms(2026, 11, 14, 8, 0, 0).unwrap()),
            last_status: Some("needs_human".into()),
        };
        let dto = SourceDto::from(&source);
        assert_eq!(
            (dto.session_id, dto.last_status.as_deref()),
            (Some(9), Some("needs_human"))
        );
        assert_eq!(
            dto.host, "boards.greenhouse.io",
            "the host, never the whole address"
        );
        assert_eq!(
            dto.last_run_at.as_deref(),
            Some("2026-11-14T08:00:00+00:00")
        );
    }

    #[test]
    fn inputs_travel_as_a_kind_and_a_value() {
        let json = serde_json::to_value(LeadInput::Url("https://a.test/1".into())).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "url", "value": "https://a.test/1" })
        );
        let back: LeadInput =
            serde_json::from_value(serde_json::json!({ "kind": "file", "value": "/tmp/jd.md" }))
                .unwrap();
        assert_eq!(back, LeadInput::File("/tmp/jd.md".into()));
        assert!(serde_json::from_value::<LeadInput>(
            serde_json::json!({ "kind": "shell", "value": "rm" })
        )
        .is_err());
    }

    #[test]
    fn an_added_lead_says_where_it_stands() {
        let mut l = lead();
        assert_eq!(
            AddedLeadDto::from(&l),
            AddedLeadDto {
                lead_id: 7,
                state: "new",
                application_id: None
            }
        );
        l.state = LeadState::Promoted;
        l.app_id = Some(12);
        assert_eq!(
            AddedLeadDto::from(&l),
            AddedLeadDto {
                lead_id: 7,
                state: "promoted",
                application_id: Some(12)
            }
        );
        l.state = LeadState::Dismissed;
        assert_eq!(AddedLeadDto::from(&l).state, "dismissed");
    }

    #[test]
    fn a_failed_prepare_carries_the_problem_to_act_on() {
        let outcome =
            PrepareOutcomeDto::new(5, Err(anyhow::anyhow!("no AI provider is configured")));
        let error = outcome.error.unwrap();
        assert_eq!(
            (outcome.lead_id, outcome.application_id, outcome.label),
            (5, None, None)
        );
        assert_eq!(error.problem.kind, "noProvider");
        assert_eq!(error.message, "no AI provider is configured");
    }

    #[test]
    fn a_search_ends_with_its_summary_or_its_error() {
        let ok = SearchFinishedDto::new(Ok(crate::engine::discover::RunSummary {
            saved: 3,
            waiting: 1,
            ..Default::default()
        }));
        assert_eq!(
            ok.summary.map(|s| (s.saved, s.waiting, s.failed)),
            Some((3, 1, 0))
        );
        assert!(ok.error.is_none());
        let failed = SearchFinishedDto::new(Err(anyhow::anyhow!("connection refused")));
        assert!(failed.summary.is_none());
        assert_eq!(failed.error.map(|e| e.problem.kind), Some("network"));
    }
}
