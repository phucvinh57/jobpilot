//! Dismissed jobs: dismiss a job whose CV was prepared, list every dismissed job, and restore one.
//!
//! ```text
//! core.dismiss_application(app_id, "too far", true).await?;  // CVs stage: "Dismiss", optionally "never this company"
//! for job in core.dismissed_jobs()? { .. }                     // Leads > Show: Dismissed, newest first
//! core.restore_lead(lead_id)?;                                 // `new` again (hidden if the preferences filter it)
//! core.restore_application(app_id)?;                           // back at the status it had
//! ```
//!
//! A dismissed job is never deleted: a dismissed lead stays a `leads` row, and a dismissed
//! application keeps its row (status `Dismissed`) and its lead (`promoted`, linked), so dedup
//! still knows the posting and discovery never adds it again. Restore does not take the company
//! off `exclude_companies`: the user does that in their preferences. A restored lead from a source
//! that the saved preferences still filter out is `new` but hidden ([`Lead::filtered_reason`],
//! as [`Core::refilter_leads`] would leave it): it shows once the preferences let it pass. A `new`
//! lead that is only hidden is not dismissed, so the dismissed list never has it.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::promote::Dismissed;
use super::refilter::filtered_reason;
use super::score::excluded_company;
use crate::store::{Application, Lead};
use crate::Core;

/// What a dismissed job was when it was dismissed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissedKind {
    /// A lead in the queue (dismissed by the user, or filtered out by the preferences).
    Lead,
    /// A job in the CVs stage (an application).
    Application,
}

impl DismissedKind {
    /// `lead` / `application`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Application => "application",
        }
    }

    /// Inverse of [`DismissedKind::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "lead" => Some(Self::Lead),
            "application" => Some(Self::Application),
            _ => None,
        }
    }
}

/// What "Restore" did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restored {
    /// The restored lead is hidden by the saved preferences (why): it is back, but not in the
    /// queue until they let it pass. `None` when it is shown, and for an application.
    pub hidden: Option<String>,
    /// Its company is still in the preferences' `exclude_companies` (Restore leaves it there):
    /// later searches keep leaving the company out.
    pub excluded: bool,
}

/// One row of the dismissed list.
#[derive(Clone, Debug, PartialEq)]
pub struct DismissedJob {
    pub kind: DismissedKind,
    /// The lead's id or the application's id, after `kind`.
    pub id: i64,
    pub company: String,
    pub role: String,
    /// The posting's address (a lead added from a file or text has a `manual:` one); `None` when
    /// unknown.
    pub url: Option<String>,
    /// Why, as the user typed it or the filter said; `None` when no reason was given.
    pub reason: Option<String>,
    pub dismissed_at: DateTime<Utc>,
}

fn reason_of(reason: Option<&str>) -> Option<String> {
    reason.map(str::trim).filter(|r| !r.is_empty()).map(str::to_string)
}

impl DismissedJob {
    fn of_lead(lead: &Lead) -> Self {
        Self {
            kind: DismissedKind::Lead,
            id: lead.id,
            company: lead.company.clone(),
            role: lead.role.clone(),
            url: Some(lead.url.clone()),
            reason: reason_of(lead.dismiss_reason.as_deref()),
            dismissed_at: lead.dismissed_at.unwrap_or(lead.found_at),
        }
    }

    fn of_application(app: &Application) -> Self {
        Self {
            kind: DismissedKind::Application,
            id: app.id,
            company: app.company.clone(),
            role: app.role.clone(),
            url: app.source.clone().or_else(|| app.apply_url.clone()),
            reason: reason_of(app.dismiss_reason.as_deref()),
            dismissed_at: app.dismissed_at.unwrap_or(app.updated_at),
        }
    }
}

impl Core {
    /// "Dismiss" a job in the CVs stage: the application moves to `Dismissed`, with an optional
    /// `reason`, and keeps its row and files (Restore brings it back). With `exclude_company` the
    /// company is also kept out of later searches and its leads in the queue are dismissed with it,
    /// as for a lead ([`Core::dismiss_lead_with_reason`]).
    ///
    /// Only an application in `Draft`, `CvReady` or `Failed` can be dismissed; not while its CV is
    /// queued or being written from its lead, nor while its application form is being opened,
    /// submitted, or is open (errors): both gates hold while the status changes, so none of those
    /// can start in between. A generate / revise job the shell runs for it is the shell's to hold
    /// off (its job slot), and the pipeline refuses a dismissed job when one begins.
    pub async fn dismiss_application(&self, app_id: i64, reason: &str, exclude_company: bool) -> Result<Dismissed> {
        let app = self.store.get_application(app_id)?.with_context(|| format!("application {app_id} no longer exists"))?;
        let lead = self.store.lead_of_application(app_id)?.map(|l| l.id);
        self.refuse_while_preparing(lead, || {
            crate::apply::while_no_run(self, app_id, || self.store.dismiss_application(app_id, reason))
        })?;
        let company = app.company.trim();
        let exclusion = exclude_company && !company.is_empty();
        tracing::info!(app = app_id, exclusion, "application dismissed");
        if !exclusion {
            return Ok(Dismissed::default());
        }
        self.exclude_company_and_sweep(company).await
    }

    /// Every dismissed job, most recently dismissed first: the leads the user dismissed or the
    /// preferences filtered out, and the applications dismissed from the CVs stage.
    pub fn dismissed_jobs(&self) -> Result<Vec<DismissedJob>> {
        let leads = self.store.list_dismissed_leads()?;
        let apps = self.store.list_dismissed_applications()?;
        let mut out: Vec<DismissedJob> =
            leads.iter().map(DismissedJob::of_lead).chain(apps.iter().map(DismissedJob::of_application)).collect();
        // Stable: within the same moment, each list keeps its own order.
        out.sort_by_key(|job| std::cmp::Reverse(job.dismissed_at));
        Ok(out)
    }

    /// How many jobs [`Core::dismissed_jobs`] lists, for the filter's count (two counts, no rows).
    pub fn dismissed_count(&self) -> Result<usize> {
        Ok(self.store.count_dismissed_leads()? + self.store.count_dismissed_applications()?)
    }

    /// "Restore" a dismissed lead: `new` again, in the queue unless the saved preferences filter it
    /// out (a lead from a source; then it is hidden, as [`Core::refilter_leads`] does, and
    /// [`Restored::hidden`] says why), in one write. Its company stays excluded if the user excluded
    /// it ([`Restored::excluded`]).
    pub fn restore_lead(&self, lead_id: i64) -> Result<Restored> {
        let lead = self.store.get_lead(lead_id)?.with_context(|| format!("lead {lead_id} no longer exists"))?;
        let prefs = self.store.preferences()?;
        let hidden = if lead.source_id.is_some() { filtered_reason(&prefs, &lead) } else { None };
        self.store.restore_lead(lead_id, hidden.as_deref())?;
        let excluded = excluded_company(&prefs, &lead.company).is_some();
        tracing::info!(lead = lead_id, hidden = hidden.is_some(), excluded, "lead restored");
        Ok(Restored { hidden, excluded })
    }

    /// "Restore" a dismissed application: back at the status it had before (a timeline event says
    /// so). [`Restored::excluded`] says whether its company is still excluded.
    pub fn restore_application(&self, app_id: i64) -> Result<Restored> {
        let status = self.store.restore_application(app_id)?;
        let company = self.store.get_application(app_id)?.map(|a| a.company).unwrap_or_default();
        let excluded = excluded_company(&self.store.preferences()?, &company).is_some();
        tracing::info!(app = app_id, status = status.as_str(), excluded, "application restored");
        Ok(Restored { hidden: None, excluded })
    }

    /// "Restore" the dismissed job `id` of `kind`.
    pub fn restore_dismissed(&self, kind: DismissedKind, id: i64) -> Result<Restored> {
        match kind {
            DismissedKind::Lead => self.restore_lead(id),
            DismissedKind::Application => self.restore_application(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::dedup;
    use crate::runtime::block_on;
    use crate::store::{LeadState, NewApplication, NewLead, NewSource, SourcePurpose, Status};
    use crate::{Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn dismiss(core: &Core, app_id: i64, reason: &str, exclude: bool) -> Result<Dismissed> {
        block_on(core.dismiss_application(app_id, reason, exclude))
    }

    fn lead(core: &Core, n: u32, company: &str) -> Lead {
        core.store
            .add_lead(&NewLead {
                url: format!("https://jobs.test/{n}"),
                dedup_key: format!("{}-role-{n}", company.to_lowercase()),
                company: company.into(),
                role: format!("Role {n}"),
                score: Some(70.0),
                ..Default::default()
            })
            .unwrap()
    }

    /// A lead prepared into a Draft application, as Prepare CV leaves it (without the CV).
    fn prepared(core: &Core, n: u32, company: &str) -> (Lead, Application) {
        let l = lead(core, n, company);
        let new = NewApplication {
            slug: format!("app-{n}"),
            company: company.into(),
            role: format!("Role {n}"),
            source: Some(l.url.clone()),
            dir: core.paths.application_dir(&format!("app-{n}")),
            ..Default::default()
        };
        let app = core.store.promote_lead_to_application(l.id, &new, None).unwrap();
        (core.store.get_lead(l.id).unwrap().unwrap(), app)
    }

    #[test]
    fn a_dismissed_application_is_listed_known_to_dedup_and_restored() {
        let (_dir, core) = core();
        let (lead_row, app) = prepared(&core, 1, "Acme");
        core.store.set_status(app.id, Status::CvReady, "approved").unwrap();

        let out = dismiss(&core, app.id, "salary too low", false).unwrap();
        assert_eq!(out, Dismissed::default());
        let jobs = core.dismissed_jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!((jobs[0].kind, jobs[0].id), (DismissedKind::Application, app.id));
        assert_eq!(jobs[0].reason.as_deref(), Some("salary too low"));
        assert_eq!(jobs[0].url.as_deref(), Some("https://jobs.test/1"));
        assert_eq!(core.dismissed_count().unwrap(), 1);

        // Its lead stays promoted and linked, so the posting is still known.
        let still = core.store.get_lead(lead_row.id).unwrap().unwrap();
        assert_eq!((still.state, still.app_id), (LeadState::Promoted, Some(app.id)));
        let known = dedup::Known::load(&core.store).unwrap();
        let url = dedup::canonical_url("https://jobs.test/1").unwrap();
        assert!(known.has_application(&url, ""), "the dismissed application is known by its posting");
        assert!(known.contains(&core.store, "https://elsewhere.test/1", &lead_row.dedup_key).unwrap());
        assert!(core.store.list_queue_leads().unwrap().is_empty());

        let out = core.restore_dismissed(DismissedKind::Application, app.id).unwrap();
        assert_eq!(out, Restored { hidden: None, excluded: false });
        assert_eq!(core.store.get_application(app.id).unwrap().unwrap().status, Status::CvReady);
        assert!(core.dismissed_jobs().unwrap().is_empty());
    }

    #[test]
    fn dismissing_an_application_can_exclude_its_company_and_sweep_the_queue() {
        let (_dir, core) = core();
        let (_, app) = prepared(&core, 1, "Acme");
        let other = lead(&core, 2, "Acme");
        let keep = lead(&core, 3, "Globex");

        let out = dismiss(&core, app.id, "", true).unwrap();
        assert_eq!((out.also, out.kept, out.not_excluded), (1, 0, None));
        assert!(core.preferences().unwrap().exclude_companies.iter().any(|c| c == "Acme"));
        assert_eq!(core.store.get_lead(other.id).unwrap().unwrap().state, LeadState::Dismissed);
        assert_eq!(core.store.get_lead(keep.id).unwrap().unwrap().state, LeadState::New);

        // Newest first, the application with no reason, the swept lead with the filter's.
        let jobs = core.dismissed_jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().any(|j| j.kind == DismissedKind::Application && j.reason.is_none()));
        assert!(jobs.iter().any(|j| j.kind == DismissedKind::Lead && j.id == other.id && j.reason.is_some()));
        assert!(jobs.windows(2).all(|w| w[0].dismissed_at >= w[1].dismissed_at));

        // Restore puts the lead back in the queue; the company stays excluded.
        let out = core.restore_dismissed(DismissedKind::Lead, other.id).unwrap();
        assert_eq!(out, Restored { hidden: None, excluded: true }, "a lead the user added is never filtered");
        assert_eq!(core.store.get_lead(other.id).unwrap().unwrap().state, LeadState::New);
        assert!(core.preferences().unwrap().exclude_companies.iter().any(|c| c == "Acme"));
        assert_eq!(core.dismissed_jobs().unwrap().len(), 1);
        assert_eq!(core.dismissed_count().unwrap(), 1);
    }

    #[test]
    fn a_restored_lead_the_preferences_still_filter_is_back_but_hidden() {
        let (_dir, core) = core();
        let source = core
            .store
            .add_source(&NewSource { kind: "lever".into(), purpose: SourcePurpose::Jobs, ..Default::default() })
            .unwrap();
        let found = |n: u32, company: &str| {
            let l = lead(&core, n, company);
            core.store.with_conn(|c| Ok(c.execute("UPDATE leads SET source_id = ?1 WHERE id = ?2", [source.id, l.id])?)).unwrap();
            l
        };
        let acme = found(1, "Acme");
        let globex = found(2, "Globex");
        let mut prefs = core.preferences().unwrap();
        prefs.exclude_companies = vec!["Acme".into()];
        core.store.save_preferences(&prefs).unwrap();
        core.store.dismiss_lead(acme.id, "excluded company: Acme").unwrap();
        core.store.dismiss_lead(globex.id, "not now").unwrap();
        // Hidden-only leads never count as dismissed.
        let hidden = found(3, "Acme");
        core.store.set_lead_filtered(hidden.id, Some("excluded company: Acme")).unwrap();
        assert_eq!(core.dismissed_count().unwrap(), 2);
        assert_eq!(core.dismissed_jobs().unwrap().len(), 2);

        let out = core.restore_lead(acme.id).unwrap();
        assert_eq!(out.hidden.as_deref(), Some("excluded company: Acme"));
        assert!(out.excluded);
        let back = core.store.get_lead(acme.id).unwrap().unwrap();
        assert_eq!((back.state, back.filtered_reason.as_deref()), (LeadState::New, Some("excluded company: Acme")));
        assert!(core.store.list_queue_leads().unwrap().iter().all(|l| l.id != acme.id), "hidden, not in the queue");

        assert_eq!(core.restore_lead(globex.id).unwrap(), Restored::default());
        assert_eq!(core.store.list_queue_leads().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), [globex.id]);
        assert_eq!(core.dismissed_count().unwrap(), 0);
    }

    #[test]
    fn only_a_job_in_cvs_can_be_dismissed() {
        let (_dir, core) = core();
        let (_, app) = prepared(&core, 1, "Acme");
        core.store.set_status(app.id, Status::Submitting, "").unwrap();
        assert!(dismiss(&core, app.id, "", false).is_err());
        core.store.set_status(app.id, Status::Applied, "").unwrap();
        assert!(dismiss(&core, app.id, "", false).is_err());
        assert!(dismiss(&core, 999, "", false).is_err());
        assert!(core.dismissed_jobs().unwrap().is_empty());
        assert!(core.restore_lead(999).is_err() && core.restore_application(app.id).is_err());
    }

    #[test]
    fn a_dismissed_job_is_never_finished_by_hand_and_its_cv_never_rewritten() {
        let (_dir, core) = core();
        let (_, app) = prepared(&core, 1, "Acme");
        dismiss(&core, app.id, "", false).unwrap();
        let err = block_on(crate::apply::finish_manually(&core, app.id, true)).unwrap_err();
        assert!(format!("{err:#}").contains("dismissed"), "{err:#}");
        assert!(block_on(crate::apply::finish_manually(&core, app.id, false)).is_err());
        let err = block_on(crate::pipeline::regenerate(&core, app.id, |_| {})).unwrap_err();
        assert!(format!("{err:#}").contains("dismissed"), "{err:#}");
        assert!(block_on(crate::pipeline::revise(&core, app.id, "shorter", |_| {})).is_err());
        assert!(block_on(crate::pipeline::rerender(&core, app.id)).is_err());
        assert_eq!(core.store.get_application(app.id).unwrap().unwrap().status, Status::Dismissed);
    }

    #[test]
    fn kinds_have_stable_names() {
        for kind in [DismissedKind::Lead, DismissedKind::Application] {
            assert_eq!(DismissedKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DismissedKind::parse("app"), None);
    }
}
