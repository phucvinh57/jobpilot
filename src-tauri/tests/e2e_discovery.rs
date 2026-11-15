//! End to end, with no real service: a job source -> leads with score and insight -> the queue ->
//! "Prepare CV" -> "Approve CV" -> (Chrome) a dry-run apply on a local form.
//!
//! Everything runs against loopback servers started here:
//! - a fake OpenAI-compatible provider (`/v1/chat/completions`) that answers by the shape of the
//!   JSON schema it is asked for, and is registered as the default provider in the throwaway data
//!   dir, so the real `OpenAiClient`, the real scoring, insight, analysis, tailoring, fact guard
//!   and apply answers all run;
//! - a Greenhouse-like board API that the real `Greenhouse` adapter reads, through the real safe
//!   client with its loopback guard switched off (`allow_private_hosts`, the test setting);
//! - a plain application form for the apply step.
//!
//! The CV is rendered by the real `typst` CLI. [`discovery_to_approved_cv`] needs only that;
//! [`discovery_to_dry_run_apply`] also starts Chrome, so it is `#[ignore]`d:
//! `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot --test e2e_discovery -- --ignored`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use jobpilot_lib::engine::apply::{self, ApplyOptions, Prepared, Submitted};
use jobpilot_lib::engine::discover::source::{Greenhouse, JobSource, NetHttp};
use jobpilot_lib::engine::discover::{DiscoverDeps, RunSummary};
use jobpilot_lib::engine::net::http::{Http, HttpLimits};
use jobpilot_lib::engine::pipeline::cv::Cv;
use jobpilot_lib::engine::pipeline::insight::Insight;
use jobpilot_lib::engine::prefs::Preferences;
use jobpilot_lib::engine::store::{LeadState, Provider, ProviderKind, Status};
use jobpilot_lib::engine::{runtime, Core, Paths, Secrets};

const EXAMPLE_CV: &str = include_str!("../../assets/prompts/example-cv.yaml");
/// The user's document: a full, real profile (every fact the tailored CV may state).
const PROFILE_DOC: &str = include_str!("fixtures/profile.yaml");
const SECTIONS: &str = "sections: [summary, skills, projects, experience, education, languages]";
/// The page budget is one page and the full example takes two: without a trim round.
fn one_page_cv() -> String {
    EXAMPLE_CV.replace(SECTIONS, "sections: [summary, skills, projects, experience, education]")
}

// ---------------------------------------------------------------- a tiny HTTP server

struct Req {
    method: String,
    path: String,
    body: String,
}

struct Resp {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl Resp {
    fn ok(content_type: &'static str, body: impl Into<String>) -> Self {
        Self { status: 200, content_type, body: body.into() }
    }

    fn json(body: &Value) -> Self {
        Self::ok("application/json", body.to_string())
    }

    fn html(body: impl Into<String>) -> Self {
        Self::ok("text/html; charset=utf-8", body)
    }

    fn not_found() -> Self {
        Self { status: 404, content_type: "text/plain", body: "not found".into() }
    }
}

type Handler = Arc<dyn Fn(Req) -> Resp + Send + Sync>;

/// Serve `handler` on a free loopback port, one request per connection.
async fn serve(handler: Handler) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buf = [0u8; 8192];
                let (head_end, length) = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        break (pos + 4, length);
                    }
                };
                while data.len() < head_end + length {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&data[..head_end]).to_string();
                let mut first = head.lines().next().unwrap_or("").split_whitespace();
                let (method, target) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("/"));
                let path = target.split('?').next().unwrap_or("/").to_string();
                let body = String::from_utf8_lossy(&data[head_end..]).to_string();
                let resp = handler(Req { method, path, body });
                let text = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp.status,
                    resp.content_type,
                    resp.body.len(),
                    resp.body
                );
                let _ = stream.write_all(text.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    addr
}

// ---------------------------------------------------------------- the fake model

/// What the fake provider was asked, in order (`score`, `insight`, `analysis`, `tailor`, `answers`).
#[derive(Clone, Default)]
struct Asked(Arc<Mutex<Vec<&'static str>>>);

impl Asked {
    fn count(&self, kind: &str) -> usize {
        self.0.lock().unwrap().iter().filter(|k| **k == kind).count()
    }

    fn all(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

/// Which request this is, from the shape of the schema it asks for.
fn kind_of(schema: &Value) -> Option<&'static str> {
    let props = &schema["properties"];
    [
        ("salary", "score"),
        ("source_pages", "insight"),
        ("slug", "analysis"),
        ("yaml", "tailor"),
        ("answers", "answers"),
        ("submitted", "confirm"),
        ("basics", "profile"),
    ]
        .into_iter()
        .find(|(key, _)| props.get(key).is_some())
        .map(|(_, kind)| kind)
}

/// The profile the reader "finds" in the document: the fixture in the reader's schema (no layout
/// keys, no `hidden` flags: the reader shows every item, and the example CV uses items the
/// fixture hides).
fn profile_reply() -> Value {
    fn unflagged(value: Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.into_iter().map(unflagged).collect()),
            Value::Object(map) => Value::Object(map.into_iter().filter(|(k, _)| k != "hidden").map(|(k, v)| (k, unflagged(v))).collect()),
            other => other,
        }
    }
    let cv = Cv::parse(PROFILE_DOC).expect("the fixture profile");
    let Value::Object(all) = serde_json::to_value(&cv).expect("as JSON") else { panic!("a CV is an object") };
    let keys = ["basics", "summary", "skills", "experience", "projects", "education", "certifications", "languages"];
    unflagged(Value::Object(all.into_iter().filter(|(k, _)| keys.contains(&k.as_str())).collect()))
}

fn score_reply(posting: &str) -> Value {
    let rust = posting.contains("Rust");
    let (skills, domain) = if rust { (90, 70) } else { (40, 30) };
    json!({
        "skills": {"score": skills, "known": true, "reason": if rust { "Rust and Go match the profile" } else { "Java stack, little overlap" }},
        "domain": {"score": domain, "known": true, "reason": "adjacent domain"},
        "location": {"score": 100, "known": true, "reason": "remote"},
        "salary": {"score": 0, "known": false, "reason": "not stated"},
        "seniority": {"score": 80, "known": true, "reason": "fits"},
        "eligible": "unclear",
        "eligible_reason": "remote, no limit stated"
    })
}

fn insight_reply() -> Value {
    json!({
        "domain": "Warehouse robotics",
        "business_summary": "Acme Robotics builds warehouse robots.",
        "company": {"size": "", "stage": "Series B", "funding": "", "hq": "", "founded": ""},
        "skills": {"required": ["Rust", "Go"], "preferred": ["Kubernetes"]},
        "interview": {"stages": [], "notes": "", "found": false},
        "source_pages": []
    })
}

fn analysis_reply() -> Value {
    json!({
        "company": "Acme Robotics", "role": "Senior Backend Engineer", "slug": "acme-robotics-senior-backend-engineer",
        "location": "Remote - Vietnam", "apply_url": "",
        "must_have": ["Rust", "Go"], "nice_to_have": ["Kubernetes"], "key_terms": ["Rust", "Go", "Kubernetes"], "seniority": "senior"
    })
}

/// Answers the apply form's questions: the profile settles the contact fields, the model only
/// writes the free-text one.
fn answers_reply(user: &str) -> Value {
    let list = user.split("<form_fields>").nth(1).and_then(|s| s.split("</form_fields>").next()).unwrap_or("[]");
    let fields: Vec<Value> = serde_json::from_str(list.trim()).unwrap_or_default();
    let answers: Vec<Value> = fields
        .iter()
        .map(|f| {
            let id = f["id"].as_str().unwrap_or_default();
            if f["label"].as_str().unwrap_or_default().to_lowercase().contains("why") {
                json!({"id": id, "value": "I like building backend services in Rust.", "source": "generated", "needs_input": false})
            } else {
                json!({"id": id, "value": "", "source": "unknown", "needs_input": true})
            }
        })
        .collect();
    json!({"answers": answers})
}

/// The fake OpenAI-compatible provider. Returns its address and the log of what it was asked.
async fn serve_model() -> (SocketAddr, Asked) {
    let asked = Asked::default();
    let log = asked.clone();
    let addr = serve(Arc::new(move |req: Req| {
        if req.method != "POST" || req.path != "/v1/chat/completions" {
            return Resp::not_found();
        }
        let body: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
        let schema = &body["response_format"]["json_schema"]["schema"];
        let user = body["messages"][1]["content"].as_str().unwrap_or("");
        let Some(kind) = kind_of(schema) else {
            return Resp { status: 400, content_type: "text/plain", body: format!("the fake model does not know this schema: {schema}") };
        };
        log.0.lock().unwrap().push(kind);
        let reply = match kind {
            "score" => score_reply(user.split("<posting>").nth(1).unwrap_or("")),
            "insight" => insight_reply(),
            "analysis" => analysis_reply(),
            "tailor" => json!({"yaml": one_page_cv(), "emphasised": ["Rust services"], "omitted": ["Hardware projects"], "gaps": ["Kubernetes at scale"]}),
            "answers" => answers_reply(user),
            "profile" => profile_reply(),
            _ => json!({"submitted": true}),
        };
        Resp::json(&json!({"choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": reply.to_string()}}]}))
    }))
    .await;
    (addr, asked)
}

// ---------------------------------------------------------------- the fake board and form

fn board_job(id: u32, title: &str, company: &str, location: &str, description: &str) -> Value {
    json!({
        "id": id, "title": title, "company_name": company, "location": {"name": location},
        // A real public address: the lead keeps it, and nothing ever requests it (the adapter reads
        // the posting from the board API).
        "absolute_url": format!("https://boards.greenhouse.io/acme/jobs/{id}"),
        "content": description.replace('<', "&lt;").replace('>', "&gt;"),
    })
}

fn board_jobs() -> Vec<Value> {
    vec![
        board_job(
            101,
            "Senior Backend Engineer",
            "Acme Robotics",
            "Remote - Vietnam",
            "<p>Acme Robotics is a Series B company that builds warehouse robots.</p><p>You will build and run backend services in Rust and Go, own their reliability, and work with a small team on Kubernetes.</p>",
        ),
        board_job(
            102,
            "Software Engineer, Platform",
            "Globex",
            "Remote",
            "<p>Globex runs billing systems for utilities.</p><p>You will maintain Java services, write integration tests, and join the on-call rota with a small team.</p>",
        ),
        board_job(103, "Account Executive, Enterprise", "Acme Robotics", "New York, NY", "<p>Sell robots to enterprises and manage a pipeline of accounts all year.</p>"),
        board_job(
            104,
            "Backend Engineer",
            "Evil Corp",
            "Remote",
            "<p>Evil Corp wants a backend engineer for services in Rust, with a small team and a long list of duties.</p>",
        ),
        json!({"id": 105, "title": "Backend Engineer", "company_name": "Odd", "location": null, "absolute_url": "javascript:alert(1)"}),
    ]
}

/// `GET /v1/boards/acme/jobs` and `/v1/boards/acme/jobs/<id>` of the Greenhouse board API.
async fn serve_board(hits: Arc<Mutex<Vec<String>>>) -> SocketAddr {
    serve(Arc::new(move |req: Req| {
        hits.lock().unwrap().push(req.path.clone());
        let jobs = board_jobs();
        match req.path.strip_prefix("/v1/boards/acme/jobs") {
            Some("") => Resp::json(&json!({"jobs": jobs, "meta": {"total": jobs.len()}})),
            Some(id) => {
                let id = id.trim_start_matches('/');
                jobs.iter().find(|j| j["id"].as_u64() == id.parse().ok()).map_or_else(Resp::not_found, Resp::json)
            }
            None => Resp::not_found(),
        }
    }))
    .await
}

const FORM: &str = r#"<!doctype html><html><head><title>Senior Backend Engineer at Acme Robotics</title></head><body>
<h1>Senior Backend Engineer</h1>
<form id="application_form" method="post" action="/submit" enctype="multipart/form-data">
 <div><label for="name">Full name *</label><input id="name" name="name" required></div>
 <div><label for="email">Email *</label><input id="email" name="email" type="email" required></div>
 <div><label for="phone">Phone</label><input id="phone" name="phone" type="tel"></div>
 <div><label for="resume">Resume/CV *</label><input id="resume" name="resume" type="file" required></div>
 <div><label for="why">Why do you want to work at Acme Robotics?</label><textarea id="why" name="why"></textarea></div>
 <button type="submit">Submit Application</button>
</form></body></html>"#;

/// The application form; every POST is recorded (a dry run must send none).
async fn serve_form(posts: Arc<Mutex<Vec<String>>>) -> SocketAddr {
    serve(Arc::new(move |req: Req| match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/apply") => Resp::html(FORM),
        ("POST", _) => {
            posts.lock().unwrap().push(req.path);
            Resp::html("<h1>Thank you for applying!</h1>")
        }
        _ => Resp::not_found(),
    }))
    .await
}

// ---------------------------------------------------------------- the flow

async fn open_core(tmp: &Path, model: SocketAddr) -> Core {
    let core = Core::open_at(Paths::new(tmp.join("jp-e2e")), Secrets::memory()).expect("open");
    core.save_provider(
        &Provider {
            id: 0,
            name: "Fake".into(),
            kind: ProviderKind::OpenAi,
            base_url: format!("http://{model}/v1"),
            model: "fake-model".into(),
            json_mode: String::new(),
            is_default: false,
        },
        None,
    )
    .expect("provider");
    // The user's profile, read by the (fake) model from one typed document, as Settings does.
    core.add_profile_text("CV", PROFILE_DOC).expect("document");
    let profile = core.read_profile_sources().await.expect("read the documents").expect("a profile");
    assert!(profile.left_out.is_empty(), "every fact is in the document: {:?}", profile.left_out);
    core
}

/// The real defaults (default provider, insight step, browser fallback), with the board's address
/// pointed at the fixture server and the loopback guard off for it.
fn deps(core: &Core, board: SocketAddr) -> DiscoverDeps {
    let mut deps = DiscoverDeps::live(core).expect("deps");
    deps.limits.host_delay = Duration::ZERO;
    deps.limits.allow_private_hosts = true;
    let http = Http::new(&HttpLimits { host_delay: Duration::ZERO, allow_private_hosts: true, ..HttpLimits::default() }).expect("client");
    let base = format!("http://{board}");
    deps.sources = Arc::new(move |source| {
        let slug = source.url.rsplit('/').next().unwrap_or("acme");
        Ok(Arc::new(Greenhouse::new(Arc::new(NetHttp::new(http.clone())), &base, slug, &source.name)) as Arc<dyn JobSource>)
    });
    deps.notify = Arc::new(|_, _| {});
    deps
}

async fn flow(with_apply: bool) {
    let tmp = tempfile::tempdir().expect("tmp");
    let (model, asked) = serve_model().await;
    let board_hits = Arc::new(Mutex::new(Vec::new()));
    let board = serve_board(board_hits.clone()).await;
    let core = open_core(tmp.path(), model).await;

    // Preferences (what the preference chat would have saved) and one job source, as Settings does.
    core.save_preferences(Preferences {
        roles: vec!["Backend Engineer".into(), "Software Engineer".into()],
        exclude_companies: vec!["Evil Corp".into()],
        ..Default::default()
    })
    .expect("preferences");
    let source = core.add_job_source("greenhouse", "Acme Robotics", "https://boards.greenhouse.io/acme", None).expect("source");

    // 1. "Search now": 5 postings on the board, 1 off-role (never a candidate), 1 with a
    //    `javascript:` link (dropped), 1 from an excluded company (stored dismissed, no model call),
    //    2 scored and summarised.
    let summary: RunSummary = core.discover_now_with(deps(&core, board)).await.expect("discovery run");
    assert_eq!((summary.saved, summary.dismissed, summary.waiting, summary.failed), (2, 1, 0, 0), "{summary:?}");
    assert_eq!(asked.count("score"), 2, "the excluded company costs no model call: {:?}", asked.all());
    assert_eq!(asked.count("insight"), 2, "{:?}", asked.all());

    let snapshot = core.queue_snapshot().expect("queue");
    let roles: Vec<&str> = snapshot.items.iter().map(|i| i.lead.role.as_str()).collect();
    assert_eq!(roles, ["Senior Backend Engineer", "Software Engineer, Platform"], "best match first");
    let (best, other) = (&snapshot.items[0], &snapshot.items[1]);
    assert!(best.lead.score > other.lead.score, "{:?} vs {:?}", best.lead.score, other.lead.score);
    assert_eq!(best.lead.company, "Acme Robotics");
    assert_eq!(best.lead.url, "https://boards.greenhouse.io/acme/jobs/101");
    assert!(best.lead.jd_path.as_ref().is_some_and(|p| p.is_file()), "the posting text is kept on disk");
    let scored = best.scored.as_ref().expect("a stored score breakdown");
    assert_eq!(scored.criteria.skills.score, 90);
    assert_eq!(scored.total, best.lead.score);
    let insight: &Insight = best.insight.as_ref().expect("a stored insight");
    assert_eq!(insight.domain, "Warehouse robotics");
    assert_eq!(insight.company.stage, "Series B", "stated in the posting, so kept");
    assert_eq!(insight.skills.required, ["Rust", "Go"]);
    let dismissed = core.store.list_leads(LeadState::Dismissed).expect("dismissed leads");
    assert_eq!(dismissed.len(), 1);
    assert!(dismissed[0].dismiss_reason.as_deref().is_some_and(|r| r.contains("Evil Corp")), "{:?}", dismissed[0].dismiss_reason);
    assert_eq!(core.store.get_source(source.id).expect("source").expect("row").last_status.as_deref(), Some("ok"));
    assert!(core.store.list_applications().expect("applications").is_empty(), "discovery never starts an application");

    // 2. A second run finds the same postings and drops them before any request or model call.
    let calls = asked.all().len();
    let again = core.discover_now_with(deps(&core, board)).await.expect("second run");
    assert_eq!((again.saved, again.dismissed), (0, 0), "{again:?}");
    assert_eq!(asked.all().len(), calls, "known postings cost nothing");

    // 3. "Prepare CV" on the best lead: an application with a CV, written from the posting.
    let lead_id = best.lead.id;
    let app = core.prepare_cv(lead_id).await.expect("prepare_cv");
    assert_eq!(app.status, Status::Draft);
    assert_eq!((app.company.as_str(), app.role.as_str()), ("Acme Robotics", "Senior Backend Engineer"));
    assert_eq!(app.apply_url.as_deref(), Some("https://boards.greenhouse.io/acme/jobs/101"), "the posting is the apply page until the user says otherwise");
    assert!(app.cv_yaml_path().is_file() && std::fs::metadata(app.cv_pdf_path()).expect("cv.pdf").len() > 1000, "typst rendered the CV");
    assert!(std::fs::read(app.cv_pdf_path()).expect("cv.pdf").starts_with(b"%PDF"));
    assert_eq!(app.insight_json, best.lead.insight_json, "the lead's insight is copied, not computed again");
    assert_eq!(asked.count("insight"), 2, "{:?}", asked.all());
    assert_eq!((asked.count("analysis"), asked.count("tailor")), (1, 1), "{:?}", asked.all());
    let lead = core.store.get_lead(lead_id).expect("lead").expect("row");
    assert_eq!((lead.state, lead.app_id), (LeadState::Promoted, Some(app.id)));
    assert_eq!(core.queue_snapshot().expect("queue").items.len(), 1, "the other lead is still waiting, never promoted by itself");
    let again = core.prepare_cv(lead_id).await.expect("prepare again");
    assert_eq!(again.id, app.id, "asking twice gives the same application");
    assert_eq!(core.store.list_applications().expect("applications").len(), 1);

    // 4. "Approve CV": the forward-only status move, with a timeline event.
    core.store.set_status(app.id, Status::CvReady, "CV approved").expect("approve");
    let events: Vec<String> = core.store.list_events(app.id).expect("events").into_iter().map(|e| e.message).collect();
    assert!(events.iter().any(|m| m.contains("CV approved")), "{events:?}");
    assert!(core.store.set_status(app.id, Status::Draft, "back").is_err(), "statuses only move forward");

    // 5. "Dismiss, never this company" on the other lead empties the queue.
    let done = core.dismiss_lead_with_reason(other.lead.id, "Java, not what I want", true).await.expect("dismiss");
    assert_eq!(done.also, 0);
    assert!(core.queue_snapshot().expect("queue").items.is_empty());
    assert!(core.preferences().expect("prefs").exclude_companies.contains(&"Globex".to_string()));

    // Everything the discovery side read came from the fixture board.
    let hits = board_hits.lock().unwrap().clone();
    assert!(hits.iter().all(|p| p.starts_with("/v1/boards/acme/jobs")), "{hits:?}");

    if !with_apply {
        return;
    }

    // 6. "Review & Submit" as a dry run on a local form: the page is filled from the profile and the
    //    model, the CV is attached, and nothing is sent.
    let posts = Arc::new(Mutex::new(Vec::new()));
    let form = serve_form(posts.clone()).await;
    core.set_apply_url(app.id, &format!("http://{form}/apply")).expect("apply url");
    let prepared = apply::prepare(&core, app.id, ApplyOptions { dry_run: true }, |_| {}).await.expect("prepare apply");
    let Prepared::Review(review) = prepared else { panic!("expected a review, got {prepared:?}") };
    assert!(review.dry_run && review.complete, "{review:?}");
    let value = |label: &str| review.rows.iter().find(|r| r.label.starts_with(label)).unwrap_or_else(|| panic!("no row {label}: {:?}", review.rows)).value.clone();
    assert!(value("Email").contains('@'), "the profile settles the contact fields");
    assert!(!value("Full name").is_empty());
    assert!(value("Why do you want").starts_with("I like building backend services in Rust"), "the model wrote the free-text answer");
    assert_eq!(review.attachment.as_deref().and_then(|p| p.file_name()).and_then(|n| n.to_str()), Some("cv.pdf"));
    assert_eq!(asked.count("answers"), 1, "{:?}", asked.all());
    let done = apply::submit(&core, app.id, Vec::new()).await.expect("submit");
    assert!(matches!(done, Submitted::DryRun), "{done:?}");
    assert!(posts.lock().unwrap().is_empty(), "a dry run sends nothing");
    assert_eq!(core.store.get_application(app.id).expect("app").expect("row").status, Status::CvReady, "a dry run does not move the status");
    apply::discard(&core, app.id).await.expect("discard");
}

#[test]
fn discovery_to_approved_cv() {
    runtime::block_on(flow(false));
}

#[test]
#[ignore = "launches Chrome"]
fn discovery_to_dry_run_apply() {
    runtime::block_on(flow(true));
}
