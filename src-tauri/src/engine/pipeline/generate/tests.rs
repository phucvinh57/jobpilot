use std::path::Path;
use std::sync::Mutex;

use serde_json::{json, Value};

use super::*;
use crate::engine::llm::fake::FakeLlm;
use crate::engine::llm::Llm;
use crate::engine::pipeline::ingest::tests::{serve, JOB_HTML};
use crate::engine::pipeline::insight::Insight;
use crate::engine::store::Status;
use crate::engine::{Paths, Secrets};

const EXAMPLE: &str = include_str!("../../../../../assets/prompts/example-cv.yaml");
const SECTIONS: &str = "sections: [summary, skills, projects, experience, education, languages]";
const JD: &str = "iCreationsLab is hiring a System Engineer to operate Linux servers, Nginx, MySQL and Cloudflare for hosting products. \
You will troubleshoot downtime, harden servers, run backups and migrations and work with Docker and CI/CD pipelines. \
Requirements: Linux administration, Nginx, DNS, SSL, scripting, English communication. Nice to have: Kubernetes, Proxmox. \
Apply at https://icreationslab.test/careers/system-engineer";

fn core() -> (tempfile::TempDir, Core) {
    crate::engine::profile::test_core_with_profile(crate::engine::profile::TEST_PROFILE)
}

fn analysis() -> Value {
    json!({
        "company": "iCreationsLab", "role": "System Engineer", "slug": "icreationslab-system-engineer",
        "location": "Ho Chi Minh City", "apply_url": "https://icreationslab.test/careers/system-engineer",
        "must_have": ["Linux administration", "Nginx"], "nice_to_have": ["Kubernetes"],
        "key_terms": ["Linux", "Nginx", "DNS"], "seniority": "mid"
    })
}

fn reply(yaml: &str, gaps: &[&str]) -> Value {
    json!({"yaml": yaml, "emphasised": ["Linux ops"], "omitted": ["Rust projects"], "gaps": gaps})
}

/// The example without the Languages section: fits on one page (the full example takes two).
fn one_page() -> String {
    EXAMPLE.replace(SECTIONS, "sections: [summary, skills, projects, experience, education]")
}

fn pngs(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("page-"))
        .count()
}

fn event_text(core: &Core, id: i64) -> String {
    core.store
        .list_events(id)
        .unwrap()
        .iter()
        .map(|e| format!("[{}] {}", e.kind, e.message))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn generate_writes_jd_cv_and_a_one_page_pdf() {
    let (_dir, core) = core();
    // analysis, first CV (2 pages), trimmed CV (1 page)
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &["Kubernetes at scale"]), reply(&one_page(), &[])]);
    let log = Mutex::new(Vec::new());
    let app =
        Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |p| log.lock().unwrap().push(p)).await.unwrap();

    assert_eq!(app.status, Status::Draft);
    assert_eq!(app.slug, "icreationslab-system-engineer");
    assert_eq!((app.company.as_str(), app.role.as_str()), ("iCreationsLab", "System Engineer"));
    assert_eq!(app.apply_url.as_deref(), Some("https://icreationslab.test/careers/system-engineer"));
    assert_eq!(app.dir, core.paths.application_dir("icreationslab-system-engineer"));
    assert!(!core.paths.application_dir("draft").exists(), "the placeholder folder was renamed");
    assert_eq!(core.store.list_applications().unwrap().len(), 1);

    let jd = std::fs::read_to_string(app.jd_path()).unwrap();
    assert!(jd.starts_with("# iCreationsLab — System Engineer\n"), "{jd}");
    assert!(jd.contains("operate Linux servers"));
    assert_eq!(read_jd_text(&app).unwrap(), JD);

    assert_eq!(std::fs::read_to_string(app.cv_yaml_path()).unwrap(), one_page().trim(), "the trimmed CV is kept");
    assert!(std::fs::metadata(app.cv_pdf_path()).unwrap().len() > 1000);
    assert_eq!(pngs(&app.dir), 1, "one page");

    let analysis: Analysis = serde_json::from_str(app.analysis_json.as_deref().unwrap()).unwrap();
    assert_eq!(analysis.key_terms, ["Linux", "Nginx", "DNS"]);
    let gaps: Vec<String> = serde_json::from_str(app.gaps_json.as_deref().unwrap()).unwrap();
    assert_eq!(gaps, ["Kubernetes at scale"], "the trim answer had no gaps, so the earlier ones are kept");

    let events = event_text(&core, app.id);
    assert!(
        events.contains("[cv] Emphasised: Linux ops") && events.contains("[cv] Omitted: Rust projects"),
        "{events}"
    );
    assert!(events.contains("[cv] Gaps: Kubernetes at scale"), "{events}");

    let log = log.lock().unwrap();
    assert!(log.iter().all(|p| p.app_id == app.id));
    let stages: Vec<Stage> = log.iter().map(|p| p.stage).collect();
    assert_eq!(stages[0], Stage::Fetching);
    assert_eq!(*stages.last().unwrap(), Stage::Done);
    for s in [Stage::Analyzing, Stage::Tailoring, Stage::Checking, Stage::Rendering] {
        assert!(stages.contains(&s), "{s:?} in {stages:?}");
    }
    assert!(log.iter().any(|p| p.detail.contains("trimming from 2 to 1")));

    let calls = llm.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls[0].user.contains("operate Linux servers"));
    assert!(calls[1].system.contains("Never invent facts") && calls[1].system.contains("ST Engineering"));
    assert!(calls[2].user.contains("must fit on 1"), "{}", calls[2].user);
}

#[tokio::test]
async fn page_budget_comes_from_settings() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &[])]);
    let app = Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    assert_eq!(llm.calls().len(), 2, "no trim round for a 2-page budget");
    assert_eq!(pngs(&app.dir), 2);
    assert_eq!(std::fs::read_to_string(app.cv_yaml_path()).unwrap(), EXAMPLE.trim());
}

#[tokio::test]
async fn a_trim_that_breaks_the_guard_keeps_the_full_cv_with_a_warning() {
    let (_dir, core) = core();
    let bad = one_page().replace("company: ST Engineering", "company: Globex");
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &[]), reply(&bad, &[])]);
    let app = Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    assert_eq!(std::fs::read_to_string(app.cv_yaml_path()).unwrap(), EXAMPLE.trim());
    assert_eq!(pngs(&app.dir), 2);
    let events = event_text(&core, app.id);
    assert!(events.contains("[warning] could not trim the CV"), "{events}");
}

#[tokio::test]
async fn invented_employer_is_repaired_once() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let bad = EXAMPLE.replace("company: ST Engineering", "company: Globex Corporation");
    let llm = FakeLlm::with_replies([analysis(), reply(&bad, &[]), reply(EXAMPLE, &[])]);
    let log = Mutex::new(Vec::new());
    let app =
        Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |p| log.lock().unwrap().push(p)).await.unwrap();
    let repair = &llm.calls()[2].user;
    assert!(repair.contains("Globex Corporation") && repair.contains("not an employer in the profile"), "{repair}");
    assert!(std::fs::read_to_string(app.cv_yaml_path()).unwrap().contains("ST Engineering"));
    assert!(log.lock().unwrap().iter().any(|p| p.detail.starts_with("repairing 1 problem")));
}

#[tokio::test]
async fn a_cv_that_stays_invalid_is_blocked_and_the_draft_keeps_the_violations() {
    let (_dir, core) = core();
    let bad =
        EXAMPLE.replace("company: ST Engineering", "company: Globex Corporation").replace("96GB RAM", "960GB RAM");
    let llm = FakeLlm::with_replies([analysis(), reply(&bad, &[]), reply(&bad, &[])]);
    let err = Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |_| {}).await.unwrap_err();
    let failed = err.downcast_ref::<GuardFailed>().expect("GuardFailed");
    assert_eq!(failed.violations.len(), 2, "{err}");
    assert_eq!(llm.calls().len(), 3, "exactly one repair round");

    let app = core.store.get_application_by_slug("icreationslab-system-engineer").unwrap().unwrap();
    assert_eq!(app.status, Status::Draft);
    assert!(app.cv_yaml_path().is_file(), "the last attempt stays editable");
    assert!(!app.cv_pdf_path().exists(), "but is not rendered");
    let events = event_text(&core, app.id);
    assert!(
        events.contains("[error]") && events.contains("Globex Corporation") && events.contains("`960`"),
        "{events}"
    );
}

#[tokio::test]
async fn unparsable_yaml_is_repaired_too() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let llm = FakeLlm::with_replies([analysis(), reply("basics: [unclosed", &[]), reply(EXAMPLE, &[])]);
    Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    assert!(llm.calls()[2].user.contains("invalid CV YAML"), "{}", llm.calls()[2].user);
}

#[tokio::test]
async fn a_typst_error_gets_one_repair_round() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let broken = EXAMPLE.replace("summary:\n", "summary:\n  - \"A #nginx thing\"\n");
    let llm = FakeLlm::with_replies([analysis(), reply(&broken, &[]), reply(EXAMPLE, &[])]);
    let app = Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    assert!(llm.calls()[2].user.contains("unknown variable: nginx"), "{}", llm.calls()[2].user);
    assert!(app.cv_pdf_path().is_file());
}

#[tokio::test]
async fn slugs_stay_unique() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &[]), analysis(), reply(EXAMPLE, &[])]);
    let p = Pipeline::new(&core, &llm);
    let a = p.generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    let b = p.generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    assert_eq!(
        (a.slug.as_str(), b.slug.as_str()),
        ("icreationslab-system-engineer", "icreationslab-system-engineer-2")
    );
    assert!(b.cv_pdf_path().is_file());
}

#[tokio::test]
async fn fetch_failures_leave_a_draft_and_a_browser_retry_finishes_it() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let spa =
        serve(200, "text/html", "<html><body><div id=root></div><script src=a.js></script></body></html>".into()).await;
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &[])]);
    let p = Pipeline::new(&core, &llm);
    let first = Mutex::new(None);
    let err = p
        .generate(JdInput::Url(spa.clone()), |pr| {
            first.lock().unwrap().get_or_insert(pr.app_id);
        })
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<crate::engine::pipeline::NeedsBrowser>().is_some(), "{err}");
    let id = first.lock().unwrap().expect("progress carries the id before the failure");
    let app = core.store.get_application(id).unwrap().unwrap();
    assert_eq!(app.status, Status::Draft);
    assert_eq!(app.role, "Job description not loaded");
    assert_eq!(app.source.as_deref(), Some(spa.as_str()));
    assert!(event_text(&core, id).contains("[error]") && event_text(&core, id).contains("needs a browser"));
    assert_eq!(llm.calls().len(), 0, "no model call for a page without text");

    // The browser renders the page; feed its HTML into the same application.
    let app = p.generate_into(id, JdInput::Html { html: JOB_HTML.into(), url: spa.clone() }, |_| {}).await.unwrap();
    assert_eq!(app.id, id);
    assert_eq!(app.slug, "icreationslab-system-engineer");
    assert!(app.cv_pdf_path().is_file());
    assert!(std::fs::read_to_string(app.jd_path()).unwrap().contains(&format!("Source: {spa}")));
    assert_eq!(core.store.list_applications().unwrap().len(), 1);
}

#[tokio::test]
async fn revise_regenerate_and_rerender() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let shorter = EXAMPLE.replace(
        "  - Familiar with Docker, Git-based deployment/CI-CD, K8s, Cloudflare, and English communication for cross-border collaboration.\n",
        "",
    );
    assert_ne!(shorter, EXAMPLE);
    let llm = FakeLlm::with_replies([
        analysis(),
        reply(EXAMPLE, &["Kubernetes at scale"]),
        // revise: no gaps in the answer -> the stored gaps stay
        reply(&shorter, &[]),
        // regenerate: reuses the stored analysis (no analysis call)
        reply(EXAMPLE, &["New gap"]),
    ]);
    let p = Pipeline::new(&core, &llm);
    let app = p.generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    core.store.set_status(app.id, Status::CvReady, "approved").unwrap();

    let app = p.revise(app.id, "drop the last summary bullet", |_| {}).await.unwrap();
    assert_eq!(std::fs::read_to_string(app.cv_yaml_path()).unwrap(), shorter.trim());
    assert_eq!(app.status, Status::CvReady, "revising does not touch the status");
    assert_eq!(app.gaps_json.as_deref(), Some("[\"Kubernetes at scale\"]"));
    let call = &llm.calls()[2];
    assert!(call.user.contains("drop the last summary bullet") && call.user.contains("Hands-on with Nginx"));
    assert!(event_text(&core, app.id).contains("[cv] CV revised: drop the last summary bullet"));
    assert!(p.revise(app.id, "  ", |_| {}).await.is_err());

    let app = p.regenerate(app.id, |_| {}).await.unwrap();
    assert_eq!(llm.calls().len(), 4);
    assert!(llm.calls()[3].user.contains("operate Linux servers"), "stored JD text is reused");
    assert_eq!(app.gaps_json.as_deref(), Some("[\"New gap\"]"));
    assert_eq!(std::fs::read_to_string(app.cv_yaml_path()).unwrap(), EXAMPLE.trim());

    // Manual edits: facts that are not in the profile are warnings, YAML/typst errors are errors.
    let edited = EXAMPLE.replace("96GB RAM", "960GB RAM");
    std::fs::write(app.cv_yaml_path(), &edited).unwrap();
    let out = p.rerender(app.id).await.unwrap();
    assert_eq!(out.violations.len(), 1, "{:?}", out.violations);
    assert!(out.violations[0].message.contains("`960`"));
    assert_eq!(out.render.page_count, 2);

    std::fs::write(app.cv_yaml_path(), "basics: [unclosed").unwrap();
    assert!(p.rerender(app.id).await.unwrap_err().to_string().contains("invalid CV YAML"));
    std::fs::write(app.cv_yaml_path(), EXAMPLE.replace("summary:\n", "summary:\n  - \"A #nginx thing\"\n")).unwrap();
    let err = p.rerender(app.id).await.unwrap_err();
    assert!(err.downcast_ref::<TypstError>().is_some(), "{err}");
    std::fs::write(app.cv_yaml_path(), one_page()).unwrap();
    let out = p.rerender(app.id).await.unwrap();
    assert!(out.violations.is_empty() && out.render.page_count == 1);
}

#[tokio::test]
async fn bad_input_creates_no_draft() {
    let (_dir, core) = core();
    let llm = FakeLlm::with_replies([]);
    let p = Pipeline::new(&core, &llm);
    assert!(p.generate(JdInput::Text("x".into()), |_| {}).await.is_err());
    assert!(p.generate(JdInput::Url("ftp://x.test/job".into()), |_| {}).await.is_err());
    assert!(core.store.list_applications().unwrap().is_empty());
    // No provider configured: the public entry point fails before creating anything.
    assert!(generate(&core, JdInput::Text(JD.into()), |_| {})
        .await
        .unwrap_err()
        .to_string()
        .contains("no AI provider"));
    assert!(core.store.list_applications().unwrap().is_empty());
}

#[tokio::test]
async fn the_stored_profile_is_unchanged_by_generate() {
    let (_dir, core) = core();
    let before = core.store.stored_profile().unwrap();
    assert!(before.is_some());
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &[])]);
    Pipeline::new(&core, &llm).generate(JdInput::Text(JD.into()), |_| {}).await.unwrap();
    assert_eq!(core.store.stored_profile().unwrap(), before);
}

/// What the model says about the company in the brief tests.
fn brief_reply() -> Value {
    json!({
        "domain": "Warehouse robotics", "business_summary": "Builds robots for warehouses.",
        "company": {"size": "", "stage": "", "funding": "", "hq": "", "founded": ""},
        "interview": {"stages": [], "notes": "", "found": false}, "source_pages": []
    })
}

/// A model that answers the brief only once its gate is opened.
struct Gated {
    gate: Arc<tokio::sync::Notify>,
    calls: std::sync::atomic::AtomicUsize,
}

impl Llm for Gated {
    fn complete_json<'a>(&'a self, _: &'a str, _: &'a str, _: &'a Value) -> crate::engine::llm::LlmFuture<'a, Value> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            self.gate.notified().await;
            Ok(brief_reply())
        })
    }

    fn list_models(&self) -> crate::engine::llm::LlmFuture<'_, Vec<String>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }
}

/// The brief runs detached on the global runtime: wait (up to 5 s) until `done` holds.
async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

fn stored_brief(core: &Core, id: i64) -> Option<Insight> {
    core.store.get_application(id).unwrap().unwrap().insight_json.as_deref().and_then(Insight::from_json)
}

#[tokio::test]
async fn the_company_brief_runs_detached_and_never_delays_the_cv() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let url = serve(200, "text/html; charset=utf-8", JOB_HTML.to_string()).await;
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &[])]);
    let gate = Arc::new(tokio::sync::Notify::new());
    let brief_llm = Arc::new(Gated { gate: gate.clone(), calls: Default::default() });
    let app =
        Pipeline::new(&core, &llm).with_insight(brief_llm.clone()).generate(JdInput::Url(url.clone()), |_| {}).await.unwrap();

    // The CV is done while the brief is still waiting for its model.
    assert_eq!(llm.calls().len(), 2, "the brief has its own client: the CV's scripted answers are untouched");
    assert!(app.cv_pdf_path().is_file());
    assert!(app.insight_json.is_none());
    eventually("the brief to start", || brief_llm.calls.load(std::sync::atomic::Ordering::SeqCst) == 1).await;
    assert!(stored_brief(&core, app.id).is_none());

    // Generating again while that brief runs (a retry) does not start a second one.
    let llm = FakeLlm::with_replies([reply(EXAMPLE, &[])]);
    let second = Arc::new(Gated { gate: gate.clone(), calls: Default::default() });
    Pipeline::new(&core, &llm).with_insight(second.clone()).regenerate(app.id, |_| {}).await.unwrap();
    assert_eq!(second.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(brief_llm.calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    gate.notify_one();
    eventually("the brief to be stored", || stored_brief(&core, app.id).is_some()).await;
    let brief = stored_brief(&core, app.id).unwrap();
    assert_eq!(brief.domain, "Warehouse robotics");
    assert_eq!(brief.skills.required, ["Linux administration", "Nginx"], "the analysis' skills");
    // The JD's own URL is the source; a local address is not a company site, so no page was read.
    assert_eq!(brief.sources, [url]);
}

#[tokio::test]
async fn a_failed_brief_never_fails_generation_and_regenerate_fills_it_in() {
    let (_dir, core) = core();
    core.store.kv_set(MAX_PAGES_KEY, "2").unwrap();
    let url = serve(200, "text/html; charset=utf-8", JOB_HTML.to_string()).await;
    let llm = FakeLlm::with_replies([analysis(), reply(EXAMPLE, &[])]);
    let down = Arc::new(FakeLlm::with_error("HTTP 503 Service Unavailable"));
    let app = Pipeline::new(&core, &llm).with_insight(down.clone()).generate(JdInput::Url(url), |_| {}).await.unwrap();
    eventually("the failing brief to run", || down.calls().len() == 1).await;
    assert!(app.insight_json.is_none() && app.cv_pdf_path().is_file());
    assert!(!event_text(&core, app.id).contains("[error]"), "{}", event_text(&core, app.id));

    // Regenerate has the stored analysis and starts the missing brief.
    let llm = FakeLlm::with_replies([reply(EXAMPLE, &[])]);
    let up = Arc::new(FakeLlm::with_replies([brief_reply()]));
    let app = Pipeline::new(&core, &llm).with_insight(up.clone()).regenerate(app.id, |_| {}).await.unwrap();
    eventually("the brief to be stored", || stored_brief(&core, app.id).is_some()).await;
    assert_eq!(up.calls().len(), 1);

    // ...and leaves an existing one alone.
    let before = core.store.get_application(app.id).unwrap().unwrap().insight_json;
    let llm = FakeLlm::with_replies([reply(EXAMPLE, &[])]);
    let unused = Arc::new(FakeLlm::with_error("must not be called"));
    let again = Pipeline::new(&core, &llm).with_insight(unused.clone()).regenerate(app.id, |_| {}).await.unwrap();
    assert!(unused.calls().is_empty());
    assert_eq!(again.insight_json, before);
}

#[tokio::test]
async fn a_failing_cv_still_reports_its_own_error() {
    // No profile yet: the CV fails with the NoProfile error, not a brief error.
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
    let llm = FakeLlm::with_replies([analysis()]);
    let brief_llm = Arc::new(FakeLlm::with_replies([brief_reply()]));
    let err = Pipeline::new(&core, &llm)
        .with_insight(brief_llm)
        .generate(JdInput::Text(JD.into()), |_| {})
        .await
        .unwrap_err();
    assert_eq!(crate::engine::health::classify(&format!("{err:#}")), crate::engine::health::Problem::NoProfile);
}

/// The UI runs these through `runtime::spawn`, which needs `Send` futures.
#[test]
fn public_futures_are_send() {
    fn send<T: Send>(_: &T) {}
    let (_dir, core) = core();
    send(&generate(&core, JdInput::Text(String::new()), |_| {}));
    send(&generate_into(&core, 1, JdInput::Text(String::new()), |_| {}));
    send(&regenerate(&core, 1, |_| {}));
    send(&revise(&core, 1, "x", |_| {}));
    send(&rerender(&core, 1));
    send(&ingest(JdInput::Text(String::new())));
}

#[test]
fn jd_header_is_stripped_when_reading() {
    assert_eq!(strip_jd_header("# A — B\nSource: https://x\n\nbody\nmore\n"), "body\nmore");
    assert_eq!(strip_jd_header("# A — B\n\nbody"), "body");
    assert_eq!(strip_jd_header("plain text"), "plain text");
}
