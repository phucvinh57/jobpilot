//! P12 - what the queue screen shows: the `new` leads best match first, with their score
//! breakdown and brief, the filters, the sources' state and what needs the user.
//!
//! ```text
//! let snap = core.queue_snapshot()?;                       // one read, off the UI thread
//! let shown = queue::filter_items(&snap.items, &filter);   // source, minimum score, location
//! for r in snap.items[0].reasons(2) { .. }                 // "skills 90: Rust and Postgres match"
//! ```
//!
//! Sorting and filtering are plain functions over the loaded items, so the screen can re-filter
//! on every keystroke without touching the database.

use anyhow::Result;

use super::score::{Criterion, Scored};
use super::state::RunSummary;
use super::WaitingSource;
use crate::pipeline::Insight;
use crate::store::{Lead, Source, SourcePurpose};
use crate::Core;

use super::promote::FailedPrepare;

/// One lead as the queue shows it: the row, plus its score breakdown and brief parsed from the
/// stored JSON (a value that cannot be read is simply absent).
#[derive(Clone, Debug)]
pub struct QueueItem {
    pub lead: Lead,
    pub scored: Option<Scored>,
    pub insight: Option<Insight>,
}

impl QueueItem {
    pub fn new(lead: Lead) -> Self {
        let scored = lead.score_json.as_deref().and_then(|j| serde_json::from_str(j).ok());
        let insight = lead.insight_json.as_deref().and_then(Insight::from_json);
        Self { lead, scored, insight }
    }

    /// The reasons that count most for the score, at most `n`.
    pub fn reasons(&self, n: usize) -> Vec<Reason> {
        self.scored.as_ref().map(|s| top_reasons(s, n)).unwrap_or_default()
    }
}

/// One line of "why this score".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason {
    /// `skills`, `domain`, `location`, `salary` or `seniority`.
    pub criterion: &'static str,
    pub score: u32,
    pub text: String,
}

/// A criterion with the weight it counted with, for the breakdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CriterionRow {
    pub criterion: &'static str,
    pub weight: u32,
    pub detail: Criterion,
}

/// All five criteria in a fixed order, with their weights.
pub fn criteria_rows(scored: &Scored) -> Vec<CriterionRow> {
    let (c, w) = (&scored.criteria, &scored.weights);
    [
        ("skills", w.skills, &c.skills),
        ("domain", w.domain, &c.domain),
        ("location", w.location, &c.location),
        ("salary", w.salary, &c.salary),
        ("seniority", w.seniority, &c.seniority),
    ]
    .into_iter()
    .map(|(criterion, weight, detail)| CriterionRow { criterion, weight, detail: detail.clone() })
    .collect()
}

/// The criteria that were judged and say why, best contribution (`weight * score`) first, at most
/// `n`. An unjudged criterion never appears: "unknown" is not a reason.
pub fn top_reasons(scored: &Scored, n: usize) -> Vec<Reason> {
    let mut rows: Vec<CriterionRow> =
        criteria_rows(scored).into_iter().filter(|r| r.detail.known && !r.detail.reason.trim().is_empty()).collect();
    rows.sort_by_key(|r| std::cmp::Reverse((u64::from(r.weight) * u64::from(r.detail.score), r.detail.score)));
    rows.into_iter().take(n).map(|r| Reason { criterion: r.criterion, score: r.detail.score, text: r.detail.reason }).collect()
}

/// Best score first; unscored leads last; the newest first among equals.
pub fn sort_leads(leads: &mut [Lead]) {
    leads.sort_by(|a, b| {
        let by_score = match (a.score, b.score) {
            (Some(x), Some(y)) => y.total_cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        by_score.then(b.found_at.cmp(&a.found_at)).then(b.id.cmp(&a.id))
    });
}

/// What the queue screen narrows the list to. The default shows everything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueFilter {
    /// Only leads found by this source.
    pub source_id: Option<i64>,
    /// Only leads scored at least this (an unscored lead has no score to compare).
    pub min_score: Option<f64>,
    /// Only leads whose location contains this text (ignoring case).
    pub location: String,
}

impl QueueFilter {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn matches(&self, lead: &Lead) -> bool {
        if self.source_id.is_some_and(|id| lead.source_id != Some(id)) {
            return false;
        }
        if self.min_score.is_some_and(|min| !lead.score.is_some_and(|s| s >= min)) {
            return false;
        }
        let wanted = self.location.trim().to_lowercase();
        wanted.is_empty() || lead.location.to_lowercase().contains(&wanted)
    }
}

/// The items that pass `filter`, in their order.
pub fn filter_items<'a>(items: &'a [QueueItem], filter: &QueueFilter) -> Vec<&'a QueueItem> {
    items.iter().filter(|i| filter.matches(&i.lead)).collect()
}

/// Everything the queue screen needs, read in one go.
#[derive(Clone, Debug)]
pub struct QueueSnapshot {
    /// Leads waiting for the user (`new`, or promoted with the application since deleted), best
    /// match first.
    pub items: Vec<QueueItem>,
    /// Leads promoted whose CV could not be written (retry or forget).
    pub failed: Vec<FailedPrepare>,
    /// The job sources, with their last status.
    pub sources: Vec<Source>,
    /// Sources paused at a login, captcha or block page.
    pub waiting: Vec<WaitingSource>,
    pub last_run: Option<RunSummary>,
}

impl Core {
    /// Read the queue screen's data. A database read: run it off the UI thread.
    pub fn queue_snapshot(&self) -> Result<QueueSnapshot> {
        let mut leads = self.store.list_queue_leads()?;
        sort_leads(&mut leads);
        Ok(QueueSnapshot {
            items: leads.into_iter().map(QueueItem::new).collect(),
            failed: self.prepare_failures()?,
            sources: self.store.list_sources(SourcePurpose::Jobs)?,
            waiting: self.discover_waiting()?,
            last_run: self.discover_last_run()?,
        })
    }

    /// The best lead left in the queue, for "Next in queue".
    pub fn queue_next(&self) -> Result<Option<Lead>> {
        let mut leads = self.store.list_queue_leads()?;
        sort_leads(&mut leads);
        Ok(leads.into_iter().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::score::Criteria;
    use crate::prefs::Weights;
    use crate::store::{NewLead, NewSource, Store};

    fn crit(score: u32, known: bool, reason: &str) -> Criterion {
        Criterion { score, known, reason: reason.into() }
    }

    fn scored() -> Scored {
        Scored {
            total: Some(78.0),
            criteria: Criteria {
                skills: crit(90, true, "Rust and Postgres match"),
                domain: crit(40, true, "payments, not your domain"),
                location: crit(100, true, "remote"),
                salary: crit(0, false, "not stated"),
                seniority: crit(70, true, ""),
            },
            weights: Weights::default(),
        }
    }

    fn lead(store: &Store, n: u32, source: Option<i64>, location: &str, score: Option<f64>) -> Lead {
        store
            .add_lead(&NewLead {
                source_id: source,
                url: format!("https://a.test/{n}"),
                dedup_key: format!("k-{n}"),
                company: "Acme".into(),
                role: "Backend Engineer".into(),
                location: location.into(),
                score,
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn the_top_reasons_are_the_judged_criteria_that_count_most_and_say_why() {
        let s = scored();
        // skills 35*90, location 15*100, domain 20*40; salary is unknown and seniority has no reason.
        let all = top_reasons(&s, 5);
        assert_eq!(all.iter().map(|r| r.criterion).collect::<Vec<_>>(), ["skills", "location", "domain"]);
        assert_eq!(top_reasons(&s, 2).len(), 2);
        assert_eq!(all[0], Reason { criterion: "skills", score: 90, text: "Rust and Postgres match".into() });
        assert!(top_reasons(&Scored { total: None, criteria: Criteria::default(), weights: Weights::default() }, 2).is_empty());

        let rows = criteria_rows(&s);
        assert_eq!(rows.iter().map(|r| (r.criterion, r.weight)).collect::<Vec<_>>(), [("skills", 35), ("domain", 20), ("location", 15), ("salary", 15), ("seniority", 15)]);
        assert!(!rows[3].detail.known);
    }

    #[test]
    fn an_item_reads_its_score_and_brief_and_survives_bad_json() {
        let store = Store::open_in_memory().unwrap();
        let mut l = lead(&store, 1, None, "Remote", Some(78.0));
        l.score_json = Some(scored().to_json().unwrap());
        l.insight_json = Some(r#"{"domain":"Payments"}"#.into());
        let item = QueueItem::new(l.clone());
        assert_eq!(item.scored.as_ref().unwrap().total, Some(78.0));
        assert_eq!(item.insight.as_ref().unwrap().domain, "Payments");
        assert_eq!(item.reasons(1)[0].criterion, "skills");

        l.score_json = Some("not json".into());
        l.insight_json = None;
        let item = QueueItem::new(l);
        assert!(item.scored.is_none() && item.insight.is_none() && item.reasons(2).is_empty());
    }

    #[test]
    fn the_queue_is_sorted_by_score_with_unscored_last_and_newest_first_among_equals() {
        let store = Store::open_in_memory().unwrap();
        let low = lead(&store, 1, None, "", Some(41.5));
        let none_old = lead(&store, 2, None, "", None);
        let high = lead(&store, 3, None, "", Some(88.0));
        let tie_old = lead(&store, 4, None, "", Some(60.0));
        let tie_new = lead(&store, 5, None, "", Some(60.0));
        let none_new = lead(&store, 6, None, "", None);
        let mut leads = vec![low.clone(), none_old.clone(), high.clone(), tie_old.clone(), tie_new.clone(), none_new.clone()];
        sort_leads(&mut leads);
        let ids: Vec<i64> = leads.iter().map(|l| l.id).collect();
        // `found_at` can be equal within a test: the larger id is the newer lead.
        assert_eq!(ids, [high.id, tie_new.id, tie_old.id, low.id, none_new.id, none_old.id]);
    }

    #[test]
    fn filters_narrow_by_source_minimum_score_and_location() {
        let store = Store::open_in_memory().unwrap();
        let a = store.add_source(&NewSource { kind: "greenhouse".into(), purpose: SourcePurpose::Jobs, ..Default::default() }).unwrap();
        let b = store.add_source(&NewSource { kind: "lever".into(), purpose: SourcePurpose::Jobs, ..Default::default() }).unwrap();
        let items: Vec<QueueItem> = [
            lead(&store, 1, Some(a.id), "Singapore", Some(90.0)),
            lead(&store, 2, Some(a.id), "Remote - EU", Some(55.0)),
            lead(&store, 3, Some(b.id), "ho chi minh city, Vietnam", Some(70.0)),
            lead(&store, 4, Some(b.id), "Remote", None),
            lead(&store, 5, None, "", Some(95.0)),
        ]
        .into_iter()
        .map(QueueItem::new)
        .collect();
        let shown = |f: &QueueFilter| filter_items(&items, f).iter().map(|i| i.lead.url.rsplit('/').next().unwrap().to_string()).collect::<Vec<_>>();

        assert!(QueueFilter::default().is_empty());
        assert_eq!(shown(&QueueFilter::default()), ["1", "2", "3", "4", "5"], "no filter keeps everything, in order");
        assert_eq!(shown(&QueueFilter { source_id: Some(a.id), ..Default::default() }), ["1", "2"]);
        assert_eq!(shown(&QueueFilter { source_id: Some(b.id), ..Default::default() }), ["3", "4"]);
        assert_eq!(shown(&QueueFilter { min_score: Some(70.0), ..Default::default() }), ["1", "3", "5"], "70 counts; unscored never passes");
        assert_eq!(shown(&QueueFilter { location: " REMOTE ".into(), ..Default::default() }), ["2", "4"], "ignoring case and spaces");
        assert_eq!(shown(&QueueFilter { location: "vietnam".into(), ..Default::default() }), ["3"]);
        let all = QueueFilter { source_id: Some(a.id), min_score: Some(60.0), location: "sing".into() };
        assert!(!all.is_empty());
        assert_eq!(shown(&all), ["1"]);
        assert!(shown(&QueueFilter { source_id: Some(999), ..Default::default() }).is_empty());
    }

    #[test]
    fn the_snapshot_lists_new_leads_best_first_and_the_next_lead_is_the_best_left() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(crate::Paths::new(dir.path().join("jp")), crate::Secrets::memory()).unwrap();
        assert!(core.queue_snapshot().unwrap().items.is_empty());
        assert!(core.queue_next().unwrap().is_none());
        let src = core.store.add_source(&NewSource { kind: "lever".into(), purpose: SourcePurpose::Jobs, ..Default::default() }).unwrap();
        core.store.add_source(&NewSource { kind: "rss".into(), purpose: SourcePurpose::Trends, ..Default::default() }).unwrap();
        let low = lead(&core.store, 1, Some(src.id), "", Some(40.0));
        let high = lead(&core.store, 2, Some(src.id), "", Some(90.0));
        let gone = lead(&core.store, 3, Some(src.id), "", Some(99.0));
        core.store.dismiss_lead(gone.id, "").unwrap();

        let snap = core.queue_snapshot().unwrap();
        assert_eq!(snap.items.iter().map(|i| i.lead.id).collect::<Vec<_>>(), [high.id, low.id], "only `new` leads");
        assert_eq!(snap.sources.len(), 1, "job sources only");
        assert!(snap.failed.is_empty() && snap.waiting.is_empty() && snap.last_run.is_none());
        assert_eq!(core.queue_next().unwrap().unwrap().id, high.id);
        core.store.dismiss_lead(high.id, "").unwrap();
        assert_eq!(core.queue_next().unwrap().unwrap().id, low.id);
    }
}
