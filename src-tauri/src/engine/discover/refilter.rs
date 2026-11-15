//! The hard filters again, over the leads already waiting, after the preferences change: a lead
//! found before the user said where they may work (`Preferences.work_from`) or which companies to
//! skip is hidden from the queue now (`Lead::filtered_reason`), with the reason discovery would
//! have given it, and shown again when a later change (an edit, an undo) lets it pass. It stays
//! `new`: hiding is not a dismissal. No model call.
//!
//! ```text
//! core.save_preferences(prefs)?;          // the preference save paths call this themselves
//! let changed = core.refilter_leads()?;   // Refiltered { hidden, shown }
//! ```
//!
//! - Only `new` leads a source found are checked: a lead the user added by hand is theirs, and the
//!   hard filters never apply to it (see [`super::manual`]).
//! - A lead whose "Prepare CV" is queued or running is left alone ([`Core::is_preparing`]).
//! - A save runs the pass only when a field the filters read changed ([`filters_changed`]).
//! - The filters at discovery time still dismiss: those postings were never shown.

use std::path::Path;

use anyhow::Result;

use super::score::{hard_filter, ineligible_reason, Scored};
use super::source::Candidate;
use crate::engine::prefs::Preferences;
use crate::engine::store::{Lead, LeadState};
use crate::engine::Core;

/// What one pass changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refiltered {
    /// Leads hidden now (or hidden with another reason).
    pub hidden: usize,
    /// Leads shown again.
    pub shown: usize,
}

/// Whether `new` differs from `old` in a field the hard filters or the eligibility rule read:
/// `exclude_companies`, `remote`, `locations`, `work_from`, `seniority`.
pub(crate) fn filters_changed(old: &Preferences, new: &Preferences) -> bool {
    old.exclude_companies != new.exclude_companies
        || old.remote != new.remote
        || old.locations != new.locations
        || old.work_from != new.work_from
        || old.seniority != new.seniority
}

impl Core {
    /// Hide the waiting leads from sources that the saved preferences now filter out, and show
    /// again the hidden ones they let pass: the hard filters on company, role, location and the
    /// saved posting text, and (when `work_from` is set) a stored score whose model answer said
    /// the posting hires only outside it ([`ineligible_reason`]). Writes only what changes; a
    /// lead that cannot be written is logged and skipped.
    pub fn refilter_leads(&self) -> Result<Refiltered> {
        let prefs = self.store.preferences()?;
        let mut checked = 0usize;
        let mut skipped = 0usize;
        let mut out = Refiltered::default();
        for lead in self.store.list_leads(LeadState::New)?.into_iter().filter(|l| l.source_id.is_some()) {
            checked += 1;
            let reason = filtered_reason(&prefs, &lead);
            if reason == lead.filtered_reason {
                continue;
            }
            // Under the "Prepare CV" check, so one cannot start in between.
            let written = self.unless_preparing(lead.id, || self.store.set_lead_filtered(lead.id, reason.as_deref()));
            match written {
                None => skipped += 1,
                Some(Ok(true)) if reason.is_some() => out.hidden += 1,
                Some(Ok(true)) => out.shown += 1,
                Some(Ok(false)) => {}
                Some(Err(e)) => tracing::warn!(lead = lead.id, "filtering a lead again failed: {e:#}"),
            }
        }
        tracing::info!(checked, hidden = out.hidden, shown = out.shown, skipped, "leads filtered again after a preferences change");
        Ok(out)
    }

    /// [`refilter_leads`](Self::refilter_leads) after a preferences save from `old` to `new`, when
    /// a field the filters read changed ([`filters_changed`]): a failure is logged and never fails
    /// the save.
    pub(crate) fn refilter_after_save(&self, old: &Preferences, new: &Preferences) {
        if !filters_changed(old, new) {
            return;
        }
        if let Err(e) = self.refilter_leads() {
            tracing::warn!("filtering the leads again after a preferences change failed: {e:#}");
        }
    }
}

/// Why the saved preferences hide `lead` from the queue, if they do: the hard filters on company,
/// role, location and the saved posting text, then the stored score's eligibility answer.
pub(super) fn filtered_reason(prefs: &Preferences, lead: &Lead) -> Option<String> {
    let jd = lead.jd_path.as_deref().map(posting_text).unwrap_or_default();
    hard_filter(prefs, &candidate_of(lead), &jd).or_else(|| {
        let scored: Scored = serde_json::from_str(lead.score_json.as_deref()?).ok()?;
        ineligible_reason(prefs, &scored)
    })
}

fn candidate_of(lead: &Lead) -> Candidate {
    Candidate {
        url: lead.url.clone(),
        title: lead.role.clone(),
        company: lead.company.clone(),
        location: lead.location.clone(),
        snippet: String::new(),
    }
}

/// The posting text of a saved `jd.md`, without its `# company - role` and `Source:` header lines;
/// empty when the file is gone or unreadable.
fn posting_text(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else { return String::new() };
    let mut rest = text.as_str();
    if rest.starts_with("# ") {
        rest = rest.split_once('\n').map_or("", |(_, r)| r);
        if rest.starts_with("Source:") {
            rest = rest.split_once('\n').map_or("", |(_, r)| r);
        }
    }
    rest.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::discover::run::attach_jd;
    use crate::engine::prefs::Preferences;
    use crate::engine::store::{NewLead, NewSource, SourcePurpose};
    use crate::engine::{Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn source(core: &Core) -> i64 {
        core.store
            .add_source(&NewSource { kind: "greenhouse".into(), name: "acme".into(), url: "https://acme.test".into(), purpose: SourcePurpose::Jobs, session_id: None })
            .unwrap()
            .id
    }

    fn lead(core: &Core, source_id: Option<i64>, n: u32, location: &str, jd: &str) -> Lead {
        let url = format!("https://acme.test/jobs/{n}");
        let lead = core
            .store
            .add_lead(&NewLead {
                source_id,
                url: url.clone(),
                dedup_key: format!("acme-role-{n}"),
                company: "Acme".into(),
                role: format!("Engineer {n}"),
                location: location.into(),
                ..Default::default()
            })
            .unwrap();
        let c = Candidate { url, title: lead.role.clone(), company: "Acme".into(), location: location.into(), snippet: String::new() };
        attach_jd(&core.store, &core.paths, lead.id, "Acme", &c, jd).unwrap();
        core.store.get_lead(lead.id).unwrap().unwrap()
    }

    /// The lead's state, and why the preferences hide it.
    fn state(core: &Core, id: i64) -> (LeadState, Option<String>) {
        let l = core.store.get_lead(id).unwrap().unwrap();
        (l.state, l.filtered_reason)
    }

    fn queue(core: &Core) -> Vec<i64> {
        let mut ids: Vec<i64> = core.store.list_queue_leads().unwrap().into_iter().map(|l| l.id).collect();
        ids.sort_unstable();
        ids
    }

    fn work_from(countries: &[&str]) -> Preferences {
        Preferences { work_from: countries.iter().map(|c| c.to_string()).collect(), ..Default::default() }
    }

    fn from_vietnam() -> Preferences {
        work_from(&["Vietnam"])
    }

    const US_REASON: &str = "location: hires in United States, you can work from Vietnam";

    #[test]
    fn saving_work_from_hides_waiting_leads_that_hire_elsewhere_and_a_later_change_shows_them_again() {
        let (_dir, core) = core();
        let src = source(&core);
        let us = lead(&core, Some(src), 1, "Remote - US", "Build things.");
        let by_jd = lead(&core, Some(src), 2, "Remote", "Build things. You must be authorized to work in the United States.");
        let apac = lead(&core, Some(src), 3, "Remote (APAC)", "Build things.");
        let plain = lead(&core, Some(src), 4, "Remote", "Build things with a team in the US and Vietnam.");
        let manual = lead(&core, None, 5, "Remote - US", "Build things.");
        let all = vec![us.id, by_jd.id, apac.id, plain.id, manual.id];
        assert_eq!(core.refilter_leads().unwrap(), Refiltered::default(), "no preferences, nothing to filter");

        core.save_preferences(from_vietnam()).unwrap();
        let hidden = (LeadState::New, Some(US_REASON.to_string()));
        assert_eq!(state(&core, us.id), hidden, "hidden, not dismissed");
        assert_eq!(state(&core, by_jd.id), hidden);
        assert_eq!(state(&core, apac.id), (LeadState::New, None));
        assert_eq!(state(&core, plain.id), (LeadState::New, None));
        assert_eq!(state(&core, manual.id), (LeadState::New, None), "a lead the user added is never filtered");
        assert_eq!(queue(&core), [apac.id, plain.id, manual.id]);
        assert_eq!(core.new_lead_count().unwrap(), 3, "the badge counts what the list shows");
        assert_eq!(core.refilter_leads().unwrap(), Refiltered::default(), "a second pass changes nothing");

        // Adding a country (the tag field adds one at a time) shows them again.
        core.save_preferences(work_from(&["Vietnam", "United States"])).unwrap();
        assert_eq!(queue(&core), all);
        assert_eq!(state(&core, us.id), (LeadState::New, None));

        // So does removing the country.
        core.save_preferences(from_vietnam()).unwrap();
        assert_eq!(state(&core, us.id), hidden);
        core.save_preferences(Preferences::default()).unwrap();
        assert_eq!(queue(&core), all);

        // And an undo.
        core.update_preferences(from_vietnam()).unwrap();
        assert_eq!(queue(&core), [apac.id, plain.id, manual.id]);
        core.undo_preferences().unwrap();
        assert_eq!(queue(&core), all);
        assert_eq!(core.store.list_leads(LeadState::Dismissed).unwrap().len(), 0, "nothing was ever dismissed");
    }

    #[test]
    fn a_stored_model_answer_that_says_not_eligible_hides_the_lead_while_it_holds() {
        let (_dir, core) = core();
        let src = source(&core);
        let answered = lead(&core, Some(src), 1, "Remote", "Build things.");
        let blind = lead(&core, Some(src), 2, "Remote", "Build things.");
        let unclear = lead(&core, Some(src), 3, "Remote", "Build things.");
        let old = lead(&core, Some(src), 4, "Remote", "Build things.");
        let no = |judged: &[&str]| Scored {
            eligible: super::super::score::Eligible::No,
            eligible_reason: "hires in Canada only".into(),
            judged_work_from: judged.iter().map(|j| j.to_string()).collect(),
            ..Scored::default()
        };
        let set_score = |id: i64, json: &str| {
            core.store.with_conn(|c| Ok(c.execute("UPDATE leads SET score_json = ?1 WHERE id = ?2", rusqlite::params![json, id])?)).unwrap();
        };
        set_score(answered.id, &no(&["Vietnam"]).to_json().unwrap());
        set_score(blind.id, &no(&[]).to_json().unwrap());
        set_score(unclear.id, &Scored::default().to_json().unwrap());
        set_score(old.id, r#"{"total":null,"criteria":{},"weights":{}}"#);

        core.save_preferences(Preferences { exclude_companies: vec!["Nobody".into()], ..Default::default() }).unwrap();
        assert_eq!(state(&core, answered.id), (LeadState::New, None), "no work_from, the model's answer does not count");

        core.save_preferences(from_vietnam()).unwrap();
        let reason = Some("location: hires in Canada, you can work from Vietnam".to_string());
        assert_eq!(state(&core, answered.id), (LeadState::New, reason), "places only, never the model's line");
        assert_eq!(state(&core, blind.id).1, None, "judged while work_from was empty");
        assert_eq!(state(&core, unclear.id).1, None);
        assert_eq!(state(&core, old.id).1, None);

        core.save_preferences(work_from(&["Vietnam", "Canada"])).unwrap();
        assert_eq!(state(&core, answered.id).1, None, "a country added since voids the answer");
    }

    #[test]
    fn a_lead_whose_cv_is_being_prepared_is_left_alone() {
        let (_dir, core) = core();
        let src = source(&core);
        let busy = lead(&core, Some(src), 1, "Remote - US", "Build things.");
        let idle = lead(&core, Some(src), 2, "Remote - US", "Build things.");
        let held = super::super::promote::Preparing::new(&core, busy.id);
        core.save_preferences(from_vietnam()).unwrap();
        assert_eq!(state(&core, busy.id), (LeadState::New, None));
        assert_eq!(state(&core, idle.id).1.as_deref(), Some(US_REASON));
        drop(held);
        assert_eq!(core.refilter_leads().unwrap(), Refiltered { hidden: 1, shown: 0 });
        assert_eq!(state(&core, busy.id).1.as_deref(), Some(US_REASON));
    }

    #[test]
    fn only_a_change_to_a_field_the_filters_read_filters_again() {
        let (_dir, core) = core();
        let src = source(&core);
        let l = lead(&core, Some(src), 1, "Remote", "Build things.");
        // A stale mark the pass would clear: it stays as long as no pass runs.
        core.store.set_lead_filtered(l.id, Some("stale")).unwrap();
        core.save_preferences(Preferences { must_skills: vec!["Rust".into()], notes: "n".into(), ..Default::default() }).unwrap();
        core.update_preferences(Preferences { must_skills: vec!["Go".into()], ..Default::default() }).unwrap();
        assert_eq!(state(&core, l.id).1.as_deref(), Some("stale"), "skills and notes are not filters");
        core.update_preferences(Preferences { must_skills: vec!["Go".into()], seniority: vec!["Senior".into()], ..Default::default() }).unwrap();
        assert_eq!(state(&core, l.id).1, None);

        let base = Preferences::default();
        assert!(!filters_changed(&base, &Preferences { roles: vec!["SRE".into()], domains: vec!["x".into()], ..base.clone() }));
        for changed in [
            Preferences { exclude_companies: vec!["Acme".into()], ..base.clone() },
            Preferences { remote: crate::engine::prefs::Remote::Remote, ..base.clone() },
            Preferences { locations: vec!["Hanoi".into()], ..base.clone() },
            Preferences { work_from: vec!["Vietnam".into()], ..base.clone() },
            Preferences { seniority: vec!["Senior".into()], ..base.clone() },
        ] {
            assert!(filters_changed(&base, &changed), "{changed:?}");
        }
    }

    #[test]
    fn a_missing_posting_file_is_an_empty_posting() {
        let (_dir, core) = core();
        let src = source(&core);
        let l = lead(&core, Some(src), 1, "Remote", "You must be authorized to work in the United States.");
        std::fs::remove_file(l.jd_path.as_ref().unwrap()).unwrap();
        core.save_preferences(from_vietnam()).unwrap();
        assert_eq!(state(&core, l.id), (LeadState::New, None));
        assert_eq!(posting_text(Path::new("/nonexistent/jd.md")), "");
    }

    #[test]
    fn the_header_of_a_saved_posting_is_not_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jd.md");
        std::fs::write(&path, "# Acme \u{2014} US Engineer\nSource: https://acme.test/us-only\n\nThe text.\n").unwrap();
        assert_eq!(posting_text(&path), "The text.");
        std::fs::write(&path, "Plain text.").unwrap();
        assert_eq!(posting_text(&path), "Plain text.");
    }
}
