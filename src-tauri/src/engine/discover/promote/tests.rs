//! "Prepare CV" tests: `llm::fake`, a real SQLite file and the real Typst; no network.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::*;
use crate::engine::discover::source::{Candidate, JobSource, SearchCtx, SearchPage};
use crate::engine::discover::{lead_dir, DiscoverDeps, DiscoverLimits};
use crate::engine::graph::ProgressEvent;
use crate::engine::health::{classify, Problem};
use crate::engine::llm::fake::FakeLlm;
use crate::engine::pipeline::generate::MAX_PAGES_KEY;
use crate::engine::prefs::Preferences;
use crate::engine::store::{NewLead, NewSource, SourcePurpose, Status};

const EXAMPLE: &str = include_str!("../../../../../assets/prompts/example-cv.yaml");
const JD: &str = "iCreationsLab is hiring a System Engineer to operate Linux servers, Nginx, MySQL and Cloudflare for hosting products. \
You will troubleshoot downtime, harden servers, run backups and migrations and work with Docker and CI/CD pipelines. \
Requirements: Linux administration, Nginx, DNS, SSL, scripting, English communication. Nice to have: Kubernetes, Proxmox.";
const INSIGHT: &str = r#"{"domain":"Hosting","business_summary":"Sells hosting.","sources":["https://icreationslab.test/jobs/1"]}"#;

fn core() -> (tempfile::TempDir, Core) {
    let (dir, core) = crate::engine::profile::test_core_with_profile(crate::engine::profile::TEST_PROFILE);
    // The example CV takes two pages: no trim round.
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    (dir, core)
}

fn analysis() -> Value {
    json!({
        "company": "iCreationsLab", "role": "System Engineer", "slug": "icreationslab-system-engineer",
        "location": "Ho Chi Minh City", "apply_url": "",
        "must_have": ["Linux administration", "Nginx"], "nice_to_have": ["Kubernetes"],
        "key_terms": ["Linux", "Nginx", "DNS"], "seniority": "mid"
    })
}

fn reply() -> Value {
    json!({"yaml": EXAMPLE, "emphasised": ["Linux ops"], "omitted": [], "gaps": []})
}

/// A `new` lead with its posting text saved the way a discovery run saves it.
fn add_lead(core: &Core, n: u32, company: &str, role: &str, score: Option<f64>) -> Lead {
    let lead = core
        .store
        .add_lead(&NewLead {
            url: format!("https://{company}.test/jobs/{n}"),
            dedup_key: format!("{company}-{n}"),
            company: company.into(),
            role: role.into(),
            location: "Remote".into(),
            score,
            insight_json: Some(INSIGHT.into()),
            ..Default::default()
        })
        .unwrap();
    let dir = lead_dir(&core.paths, lead.id);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("jd.md");
    std::fs::write(&path, format!("# {company} — {role}\nSource: {}\n\n{JD}\n", lead.url)).unwrap();
    core.store.set_lead_jd_path(lead.id, &path).unwrap();
    core.store.get_lead(lead.id).unwrap().unwrap()
}

/// `Core::dismiss_lead_with_reason` for the tests that are not async.
fn dismiss(core: &Core, id: i64, reason: &str, exclude: bool) -> Result<Dismissed> {
    crate::engine::runtime::block_on(core.dismiss_lead_with_reason(id, reason, exclude))
}

#[test]
fn promote_is_idempotent_and_copies_the_brief() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "Acme", "Backend Engineer", Some(80.0));

    let first = core.promote(lead.id).unwrap();
    let second = core.promote(lead.id).unwrap();
    assert_eq!(first.id, second.id, "a second call returns the same application");
    assert_eq!(core.store.list_applications().unwrap().len(), 1, "never a second application");

    assert_eq!(first.status, Status::Draft);
    assert_eq!((first.company.as_str(), first.role.as_str()), ("Acme", "Backend Engineer"));
    assert_eq!(first.source.as_deref(), Some(lead.url.as_str()));
    assert_eq!(first.insight_json.as_deref(), Some(INSIGHT), "the brief is copied, not written again");
    let got = core.store.get_lead(lead.id).unwrap().unwrap();
    assert_eq!((got.state, got.app_id), (LeadState::Promoted, Some(first.id)));
    assert_eq!(core.store.list_events(first.id).unwrap().iter().filter(|e| e.kind == "created").count(), 1);

    // The store call is the same: asking again changes nothing.
    let again = core.store.promote_lead_to_application(lead.id, &NewApplication { slug: "other".into(), ..Default::default() }, None).unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(core.store.list_applications().unwrap().len(), 1);
}

#[test]
fn a_lead_added_from_text_has_no_web_address_to_apply_at() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "Acme", "Backend Engineer", None);
    core.store.with_conn(|c| Ok(c.execute("UPDATE leads SET url = 'manual:0123456789abcdef' WHERE id = ?1", [lead.id])?)).unwrap();
    let app = core.promote(lead.id).unwrap();
    assert_eq!((app.source, app.apply_url), (None, None), "a `manual:` address is not a link");
    assert!(keep_apply_url(&core, app.id, "manual:0123456789abcdef").is_ok());
    assert_eq!(core.store.get_application(app.id).unwrap().unwrap().apply_url, None);
}

#[test]
fn only_a_new_lead_with_a_readable_posting_is_promoted() {
    let (_dir, core) = core();
    let dismissed = add_lead(&core, 1, "Acme", "Backend Engineer", None);
    dismiss(&core, dismissed.id, "", false).unwrap();
    assert!(core.promote(dismissed.id).unwrap_err().to_string().contains("dismissed"));

    // No posting text: nothing is created and the lead stays in the queue.
    let no_text = core.store.add_lead(&NewLead { url: "https://x.test/1".into(), company: "X".into(), ..Default::default() }).unwrap();
    assert!(core.promote(no_text.id).is_err());
    let gone = add_lead(&core, 2, "Foo", "SRE", None);
    std::fs::remove_file(gone.jd_path.as_ref().unwrap()).unwrap();
    assert!(core.promote(gone.id).is_err());
    for id in [no_text.id, gone.id] {
        assert_eq!(core.store.get_lead(id).unwrap().unwrap().state, LeadState::New);
    }
    assert!(core.store.list_applications().unwrap().is_empty());
    assert!(core.promote(9999).is_err());
}

#[tokio::test]
async fn prepare_cv_resumes_at_generate_after_a_failure_without_a_second_application() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "iCreationsLab", "System Engineer", Some(88.0));
    let thread = prepare_thread(lead.id);
    let mut progress = core.graphs.subscribe();

    // The network is down when the CV is written.
    let down: Arc<dyn Llm> = Arc::new(FakeLlm::with_error("HTTP 503 Service Unavailable"));
    let err = core.prepare_cv_with(down, lead.id).await.unwrap_err();
    assert_eq!(classify(&format!("{err:#}")), Problem::Network, "{err:#}");

    // The application exists already (a Draft without CV), the thread failed at `generate`.
    let apps = core.store.list_applications().unwrap();
    assert_eq!(apps.len(), 1);
    let app_id = apps[0].id;
    assert!(!apps[0].cv_pdf_path().exists());
    let got = core.store.get_lead(lead.id).unwrap().unwrap();
    assert_eq!((got.state, got.app_id), (LeadState::Promoted, Some(app_id)));
    let ThreadStatus::Failed(failure) = core.graphs.status(&thread).unwrap().unwrap().status else {
        panic!("the thread should be failed")
    };
    assert_eq!((failure.node.as_str(), failure.problem), ("generate", Problem::Network));
    let failed = core.prepare_failures().unwrap();
    assert_eq!((failed.len(), failed[0].lead.id), (1, lead.id));

    // Retry with a working model: resumes at `generate`, the application is the same one.
    let llm = Arc::new(FakeLlm::with_replies([analysis(), reply()]));
    let working: Arc<dyn Llm> = llm.clone();
    let app = core.prepare_cv_with(working, lead.id).await.unwrap();
    assert_eq!(app.id, app_id);
    assert_eq!(core.store.list_applications().unwrap().len(), 1, "no second application");
    assert_eq!(core.store.list_events(app_id).unwrap().iter().filter(|e| e.kind == "created").count(), 1);
    assert_eq!(llm.calls().len(), 2, "analysis and CV");
    assert!(app.cv_yaml_path().is_file() && app.cv_pdf_path().is_file());
    assert_eq!(app.status, Status::Draft, "approving the CV is the user's call");
    assert_eq!((app.company.as_str(), app.role.as_str()), ("iCreationsLab", "System Engineer"));
    assert_eq!(app.insight_json.as_deref(), Some(INSIGHT));
    assert_eq!(app.apply_url.as_deref(), Some(lead.url.as_str()), "the posting is the apply page when the JD names none");
    assert!(matches!(core.graphs.status(&thread).unwrap().unwrap().status, ThreadStatus::Done));
    assert!(core.prepare_failures().unwrap().is_empty());

    // `promote` ran once; `generate` ran twice (the failure and the retry); progress was forwarded.
    let mut started = Vec::new();
    let mut details = Vec::new();
    while let Ok(p) = progress.try_recv() {
        assert_eq!((p.thread.as_str(), p.graph.as_str()), (thread.as_str(), PREPARE_GRAPH));
        match p.event {
            ProgressEvent::NodeStarted { node } => started.push(node),
            ProgressEvent::Detail { node, text } => {
                assert_eq!(node, "generate");
                details.push(text);
            }
            _ => {}
        }
    }
    assert_eq!(started, ["promote", "generate", "generate"]);
    assert!(details.iter().any(|d| d == "Fetching the job description"), "{details:?}");
    assert!(details.iter().any(|d| d == "Rendering the PDF"), "{details:?}");

    // Asking again for a lead that is done does not write another CV.
    let idle = Arc::new(FakeLlm::with_replies([]));
    let again: Arc<dyn Llm> = idle.clone();
    assert_eq!(core.prepare_cv_with(again, lead.id).await.unwrap().id, app_id);
    assert!(idle.calls().is_empty());
}

#[tokio::test]
async fn no_provider_fails_before_anything_is_created() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "Acme", "Backend Engineer", Some(50.0));
    let err = core.prepare_cv(lead.id).await.unwrap_err();
    assert_eq!(classify(&format!("{err:#}")), Problem::NoProvider, "{err:#}");
    assert!(core.store.list_applications().unwrap().is_empty());
    assert_eq!(core.store.get_lead(lead.id).unwrap().unwrap().state, LeadState::New, "still in the queue");
    assert!(core.graphs.status(&prepare_thread(lead.id)).unwrap().is_none());
}

#[tokio::test]
async fn several_leads_are_prepared_one_after_the_other_and_one_failing_does_not_stop_the_rest() {
    let (_dir, core) = core();
    let a = add_lead(&core, 1, "Acme", "Backend Engineer", Some(90.0));
    let b = add_lead(&core, 2, "Foo", "Platform Engineer", Some(70.0));
    let llm = Arc::new(FakeLlm::with_replies([analysis(), reply(), analysis(), reply()]));
    let shared: Arc<dyn Llm> = llm.clone();

    let missing = 9999;
    let mut results = Vec::new();
    for id in [a.id, missing, b.id] {
        results.push((id, core.prepare_cv_with(shared.clone(), id).await));
    }
    assert!(results[0].1.is_ok() && results[1].1.is_err() && results[2].1.is_ok());
    let apps = core.store.list_applications().unwrap();
    assert_eq!(apps.len(), 2, "one application per lead");
    assert!(apps.iter().all(|a| a.cv_pdf_path().is_file()));
    assert_ne!(results[0].1.as_ref().unwrap().id, results[2].1.as_ref().unwrap().id);
    for id in [a.id, b.id] {
        assert_eq!(core.store.get_lead(id).unwrap().unwrap().state, LeadState::Promoted);
    }
}

#[test]
fn dismissing_can_feed_the_preferences_back_but_never_promotes() {
    let (_dir, core) = core();
    let one = add_lead(&core, 1, "Acme", "Backend Engineer", Some(80.0));
    let two = add_lead(&core, 2, "Acme Inc", "SRE", Some(70.0));
    let other = add_lead(&core, 3, "Foo", "SRE", Some(60.0));
    let more = add_lead(&core, 4, "Metabase", "Engineer", Some(50.0));

    // A plain dismissal keeps the reason and touches nothing else.
    assert_eq!(dismiss(&core, more.id, "  too junior ", false).unwrap(), Dismissed::default());
    let got = core.store.get_lead(more.id).unwrap().unwrap();
    assert_eq!((got.state, got.dismiss_reason.as_deref()), (LeadState::Dismissed, Some("too junior")));
    assert!(core.preferences().unwrap().exclude_companies.is_empty());

    // "Don't show this company again": the preference grows and the company's other leads go too.
    assert_eq!(dismiss(&core, one.id, "not interested", true).unwrap(), Dismissed { also: 1, ..Default::default() });
    assert_eq!(core.preferences().unwrap().exclude_companies, ["Acme"]);
    let got = core.store.get_lead(two.id).unwrap().unwrap();
    assert_eq!((got.state, got.dismiss_reason.as_deref()), (LeadState::Dismissed, Some("excluded company: Acme")));
    assert_eq!(core.store.get_lead(other.id).unwrap().unwrap().state, LeadState::New);

    // Existing preferences are kept; an excluded company is not listed twice.
    core.save_preferences(Preferences { exclude_companies: vec!["acme".into(), "Evil".into()], ..core.preferences().unwrap() }).unwrap();
    let third = add_lead(&core, 5, "ACME", "Dev", None);
    dismiss(&core, third.id, "", true).unwrap();
    assert_eq!(core.preferences().unwrap().exclude_companies, ["acme", "Evil"]);

    // A lead that is no longer new cannot be dismissed, and then the preferences stay as they were.
    let before = core.preferences().unwrap();
    assert!(dismiss(&core, one.id, "again", true).is_err());
    assert_eq!(core.preferences().unwrap(), before);

    assert!(core.store.list_applications().unwrap().is_empty(), "dismissing never promotes");
}

#[test]
fn dismissing_a_company_leaves_the_leads_whose_cv_is_queued_or_being_written() {
    let (_dir, core) = core();
    let one = add_lead(&core, 1, "Acme", "Backend Engineer", Some(80.0));
    let busy = add_lead(&core, 2, "Acme", "SRE", Some(70.0));
    let idle = add_lead(&core, 3, "Acme", "Data Engineer", Some(60.0));

    let held = Preparing::new(&core, busy.id);
    assert!(core.is_preparing(busy.id) && !core.is_preparing(idle.id));
    let out = dismiss(&core, one.id, "", true).unwrap();
    assert_eq!(out, Dismissed { also: 1, kept: 1, not_excluded: None }, "the one waiting for its CV stays");
    let state = |id| core.store.get_lead(id).unwrap().unwrap().state;
    assert_eq!((state(one.id), state(busy.id), state(idle.id)), (LeadState::Dismissed, LeadState::New, LeadState::Dismissed));
    assert_eq!(core.preferences().unwrap().exclude_companies, ["Acme"], "the company is still excluded for later runs");

    // The lead itself cannot be dismissed while its CV is on the way.
    let err = dismiss(&core, busy.id, "", false).unwrap_err();
    assert!(err.to_string().contains("being prepared"), "{err:#}");
    assert_eq!(state(busy.id), LeadState::New);

    // Two holders (a batch and a retry) keep it busy until both are done.
    let second = Preparing::new(&core, busy.id);
    drop(held);
    assert!(core.is_preparing(busy.id));
    drop(second);
    assert!(!core.is_preparing(busy.id));
    dismiss(&core, busy.id, "", false).unwrap();
    assert_eq!(state(busy.id), LeadState::Dismissed);
}

#[tokio::test]
async fn a_batch_holds_all_its_leads_until_each_one_is_done() {
    let (_dir, core) = core();
    let a = add_lead(&core, 1, "Acme", "Backend Engineer", Some(90.0));
    let b = add_lead(&core, 2, "Foo", "Platform Engineer", Some(70.0));
    let llm: Arc<dyn Llm> = Arc::new(FakeLlm::with_replies([analysis(), reply(), analysis(), reply()]));

    let mut seen = Vec::new();
    core.prepare_cvs_each_with(llm, &[a.id, b.id], |id, result| {
        assert!(result.is_ok());
        // The first is done and released, the second still waits its turn.
        seen.push((id, core.is_preparing(a.id), core.is_preparing(b.id)));
    })
    .await;
    assert_eq!(seen, [(a.id, false, true), (b.id, false, false)]);
}

#[tokio::test]
async fn a_failed_prepare_leaves_a_draft_that_generate_can_finish_and_keeps_its_apply_page() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "iCreationsLab", "System Engineer", Some(88.0));

    // The model's CV breaks the fact rules twice: the analysis was adopted, then the run failed.
    let bad = json!({"yaml": "not: a cv", "emphasised": [], "omitted": [], "gaps": []});
    let llm: Arc<dyn Llm> = Arc::new(FakeLlm::with_replies([analysis(), bad.clone(), bad]));
    assert!(core.prepare_cv_with(llm, lead.id).await.is_err());
    let app = core.store.list_applications().unwrap().remove(0);
    assert_eq!(core.store.list_applications().unwrap().len(), 1);
    assert!(app.analysis_json.is_some() && app.gaps_json.is_none(), "stopped before the result was saved");
    assert_eq!(app.apply_url.as_deref(), Some(lead.url.as_str()), "the Draft still has somewhere to apply");

    // The posting text is the Draft's own `jd.md`, laid out like any application's.
    let jd = std::fs::read_to_string(app.jd_path()).unwrap();
    assert!(jd.starts_with("# iCreationsLab — System Engineer\nSource: "), "{jd}");
    assert!(read_jd_text(&app).unwrap().starts_with("iCreationsLab is hiring"));

    // The sidebar's Generate (regenerate) works on that Draft.
    let llm = FakeLlm::with_replies([reply()]);
    let done = Pipeline::new(&core, &llm).regenerate(app.id, |_| {}).await.unwrap();
    assert!(done.cv_yaml_path().is_file() && done.cv_pdf_path().is_file());
}

#[test]
fn promote_writes_the_posting_as_jd_md_and_puts_it_back_if_it_is_lost() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "Acme", "Backend Engineer", Some(80.0));
    let app = core.promote(lead.id).unwrap();
    let want = format!("# Acme — Backend Engineer\nSource: {}\n\n{JD}\n", lead.url);
    assert_eq!(std::fs::read_to_string(app.jd_path()).unwrap(), want);

    // A crash after the application was created but before the file was written.
    std::fs::remove_file(app.jd_path()).unwrap();
    let again = core.promote(lead.id).unwrap();
    assert_eq!(again.id, app.id);
    assert_eq!(std::fs::read_to_string(app.jd_path()).unwrap(), want);
}

#[tokio::test]
async fn files_without_a_saved_result_are_not_a_finished_cv_and_the_apply_page_is_kept_on_resume() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "iCreationsLab", "System Engineer", Some(88.0));
    let down: Arc<dyn Llm> = Arc::new(FakeLlm::with_error("HTTP 503 Service Unavailable"));
    assert!(core.prepare_cv_with(down, lead.id).await.is_err());
    let app = core.store.list_applications().unwrap().remove(0);

    // A run that died after rendering but before saving its result left CV files behind.
    std::fs::write(app.cv_yaml_path(), "stale: true\n").unwrap();
    std::fs::write(app.cv_pdf_path(), b"%PDF stale").unwrap();
    let llm = Arc::new(FakeLlm::with_replies([analysis(), reply()]));
    let shared: Arc<dyn Llm> = llm.clone();
    let done = core.prepare_cv_with(shared, lead.id).await.unwrap();
    assert_eq!(llm.calls().len(), 2, "the pipeline ran: the files were not taken for a result");
    assert_eq!(std::fs::read_to_string(done.cv_yaml_path()).unwrap().trim_end(), EXAMPLE.trim_end());
    assert!(done.gaps_json.is_some());

    // A thread that failed after the result was saved: resuming only repairs the apply page.
    let lead = add_lead(&core, 2, "iCreationsLab", "Site Reliability Engineer", Some(70.0));
    let down: Arc<dyn Llm> = Arc::new(FakeLlm::with_error("HTTP 503 Service Unavailable"));
    assert!(core.prepare_cv_with(down, lead.id).await.is_err());
    let app = core.store.get_application(core.store.get_lead(lead.id).unwrap().unwrap().app_id.unwrap()).unwrap().unwrap();
    let produced = FakeLlm::with_replies([analysis(), reply()]);
    let mut app = Pipeline::new(&core, &produced).generate_into(app.id, JdInput::Text(JD.into()), |_| {}).await.unwrap();
    assert!(app.gaps_json.is_some());
    app.apply_url = None;
    core.store.save_application(&app).unwrap();
    assert!(matches!(core.graphs.status(&prepare_thread(lead.id)).unwrap().unwrap().status, ThreadStatus::Failed(_)));

    let idle = Arc::new(FakeLlm::with_replies([]));
    let shared: Arc<dyn Llm> = idle.clone();
    let resumed = core.prepare_cv_with(shared, lead.id).await.unwrap();
    assert!(idle.calls().is_empty(), "the saved CV is not written again");
    assert_eq!(resumed.id, app.id);
    assert_eq!(resumed.apply_url.as_deref(), Some(lead.url.as_str()), "the fix-up runs on resume too");
    assert!(matches!(core.graphs.status(&prepare_thread(lead.id)).unwrap().unwrap().status, ThreadStatus::Done));
}

#[tokio::test]
async fn a_lead_whose_application_was_deleted_can_be_prepared_again() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "iCreationsLab", "System Engineer", Some(88.0));
    let down: Arc<dyn Llm> = Arc::new(FakeLlm::with_error("HTTP 503 Service Unavailable"));
    assert!(core.prepare_cv_with(down, lead.id).await.is_err());
    let first = core.store.list_applications().unwrap().remove(0);

    // The user deletes the Draft: the lead is promoted with nothing behind it, and is back in the queue.
    core.store.delete_application(first.id).unwrap();
    let orphan = core.store.get_lead(lead.id).unwrap().unwrap();
    assert_eq!((orphan.state, orphan.app_id), (LeadState::Promoted, None));
    assert!(orphan.in_queue());
    assert_eq!(core.queue_snapshot().unwrap().items.iter().map(|i| i.lead.id).collect::<Vec<_>>(), [lead.id]);
    assert_eq!(core.queue_next().unwrap().unwrap().id, lead.id);

    // The failed thread still points at the deleted application: it is dropped and the lead starts over.
    let llm: Arc<dyn Llm> = Arc::new(FakeLlm::with_replies([analysis(), reply()]));
    let second = core.prepare_cv_with(llm, lead.id).await.unwrap();
    assert_ne!(second.id, first.id);
    assert!(second.cv_pdf_path().is_file());
    assert_eq!(core.store.list_applications().unwrap().len(), 1);
    let got = core.store.get_lead(lead.id).unwrap().unwrap();
    assert_eq!((got.state, got.app_id), (LeadState::Promoted, Some(second.id)));
    assert!(core.queue_snapshot().unwrap().items.is_empty(), "linked again: out of the queue");

    // A finished one deleted afterwards starts over too, and an orphan can also be dismissed.
    core.store.delete_application(second.id).unwrap();
    let again = core.promote(lead.id).unwrap();
    assert_ne!(again.id, second.id);
    assert!(again.jd_path().is_file());
    core.store.delete_application(again.id).unwrap();
    core.dismiss_lead_with_reason(lead.id, "no longer wanted", false).await.unwrap();
    assert_eq!(core.store.get_lead(lead.id).unwrap().unwrap().state, LeadState::Dismissed);
    assert!(core.queue_snapshot().unwrap().items.is_empty());
}

#[test]
fn the_lead_is_dismissed_first_and_a_failing_exclusion_is_reported_not_hidden() {
    let (_dir, core) = core();
    let lead = add_lead(&core, 1, "Acme", "Backend Engineer", Some(80.0));
    let other = add_lead(&core, 2, "Acme", "SRE", Some(70.0));
    core.store
        .with_conn(|c| {
            c.execute("INSERT INTO preferences (id, json, updated_at) VALUES (1, 'not json', 'x')", [])?;
            Ok(())
        })
        .unwrap();

    let out = dismiss(&core, lead.id, "no", true).unwrap();
    assert!(out.not_excluded.as_deref().is_some_and(|e| e.contains("not valid JSON")), "{out:?}");
    assert_eq!((out.also, out.kept), (0, 0));
    let state = |id| core.store.get_lead(id).unwrap().unwrap().state;
    assert_eq!(state(lead.id), LeadState::Dismissed, "the user's dismissal is never lost");
    assert_eq!(state(other.id), LeadState::New, "nothing else changed");
}

#[test]
fn the_company_exclusion_waits_for_an_edit_in_progress_and_neither_is_lost() {
    use std::sync::mpsc;
    use std::time::Duration;

    let (_dir, core) = core();
    let core = Arc::new(core);
    let lead = add_lead(&core, 1, "Acme", "Backend Engineer", Some(80.0));

    crate::engine::runtime::block_on(async {
        let (reached, wait_reached) = mpsc::channel::<()>();
        let (go, wait_go) = mpsc::channel::<()>();
        // A chip click: it holds the edit lock and has read the document when the dismissal starts.
        let edit = tokio::spawn({
            let core = core.clone();
            async move {
                core.edit_preferences(move |p| {
                    p.locations.push("Hanoi".into());
                    reached.send(()).ok();
                    wait_go.recv_timeout(Duration::from_secs(10)).ok();
                    true
                })
                .await
            }
        });
        while wait_reached.try_recv().is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let dismissal = tokio::spawn({
            let core = core.clone();
            async move { core.dismiss_lead_with_reason(lead.id, "", true).await }
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(core.preferences().unwrap().exclude_companies.is_empty(), "the exclusion waits for the edit");
        go.send(()).unwrap();
        edit.await.unwrap().unwrap();
        let out = dismissal.await.unwrap().unwrap();
        assert!(out.not_excluded.is_none(), "{out:?}");
    });
    let prefs = core.preferences().unwrap();
    assert_eq!((prefs.locations.as_slice(), prefs.exclude_companies.as_slice()), (&["Hanoi".to_string()][..], &["Acme".to_string()][..]));
}

/// Two postings, found by a fake source.
struct Board;

#[async_trait]
impl JobSource for Board {
    async fn search(&self, _: &Preferences, _: &SearchCtx) -> Result<SearchPage> {
        let job = |n: u32, company: &str| Candidate {
            url: format!("https://board.test/jobs/{n}"),
            title: "Backend Engineer".into(),
            company: company.into(),
            location: "Remote".into(),
            snippet: String::new(),
        };
        Ok(SearchPage { candidates: vec![job(1, "Acme"), job(2, "Foo")], next: None })
    }

    async fn posting(&self, _: &Candidate) -> Result<Option<String>> {
        Ok(Some(JD.to_string()))
    }
}

#[tokio::test]
async fn a_discovery_run_fills_the_queue_and_promotes_nothing() {
    let (_dir, core) = core();
    core.store
        .add_source(&NewSource { kind: "fake".into(), name: "board".into(), url: "https://board.test".into(), purpose: SourcePurpose::Jobs, session_id: None })
        .unwrap();
    let score = json!({"score": 80, "known": true, "reason": "fits"});
    let scores = json!({"skills": score, "domain": score, "location": score, "salary": score, "seniority": score});
    let llm: Arc<dyn Llm> = Arc::new(FakeLlm::with_replies(std::iter::repeat_n(scores, 10)));
    let mut deps = DiscoverDeps::new(llm, DiscoverLimits { host_delay: std::time::Duration::ZERO, ..DiscoverLimits::default() }).unwrap();
    let searched = Arc::new(AtomicUsize::new(0));
    let counter = searched.clone();
    deps.sources = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Board))
    });

    let summary = core.discover_now_with(deps).await.unwrap();
    assert_eq!(summary.saved, 2);
    assert!(searched.load(Ordering::SeqCst) > 0);

    let snapshot = core.queue_snapshot().unwrap();
    assert_eq!(snapshot.items.len(), 2, "both leads wait in the queue");
    assert!(snapshot.items.iter().all(|i| i.lead.state == LeadState::New && i.scored.is_some()));
    assert!(core.store.list_applications().unwrap().is_empty(), "nothing is promoted without the user's Prepare CV");
    assert!(core.graphs.status(&prepare_thread(snapshot.items[0].lead.id)).unwrap().is_none());
}
