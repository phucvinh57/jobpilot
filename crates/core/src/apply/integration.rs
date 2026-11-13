//! End-to-end tests on a real headless Chrome against local fixture pages (a Greenhouse-like form,
//! a two-step form, a CAPTCHA page, a form that rejects its submission, a confirmation without clear
//! text). They are `#[ignore]`d because they start Chrome; run with
//! `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-core -- --ignored apply_` (set `JOBPILOT_CHROME` when
//! Chrome is elsewhere). Nothing leaves localhost and no real application is ever submitted.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::flow::{self, test_page};
use super::form::eval;
use super::fields::{FieldKind, Source};
use super::{ApplyOptions, Edit, Prepared, Submitted};
use crate::llm::{Llm, LlmFuture};
use crate::store::{NewApplication, Status};
use crate::{runtime, Core, Paths, Secrets};

/// Chrome tests share one process-wide registry and the dry-run env var: run them one at a time.
static LOCK: Mutex<()> = Mutex::new(());

const PROFILE: &str = "\
basics:
  name: Vinh Nguyen Phuc
  headline: Software Engineer
  email: vinh@example.com
  phone: \"0373000000\"
  location: Ho Chi Minh City, Vietnam
  github: github.com/phucvinh57
  linkedin: linkedin.com/in/npvinh57
  website: \"\"
summary:
  - 4+ years of backend work in Rust and Go
";

const GREENHOUSE: &str = r#"<!doctype html><html><head><title>Backend Engineer at Acme</title></head><body>
<h1>Backend Engineer</h1><p>Apply for this job</p>
<form id="application_form" method="post" action="/submit" enctype="multipart/form-data">
 <div class="field"><label for="first_name">First Name <span>*</span></label><input id="first_name" name="first_name" type="text" required></div>
 <div class="field"><label for="last_name">Last Name *</label><input id="last_name" name="last_name" required></div>
 <div class="field"><label for="email">Email *</label><input id="email" name="email" type="email" required></div>
 <div class="field"><label for="phone">Phone</label><input id="phone" name="phone" type="tel"></div>
 <div class="field"><label for="resume">Resume/CV *</label><input id="resume" name="resume" type="file" required></div>
 <div class="field"><label for="country">Country *</label><select id="country" name="country" required>
   <option value="">Select...</option><option value="vn">Vietnam</option><option value="us">United States</option></select></div>
 <fieldset><legend>Are you authorized to work in Vietnam? *</legend>
   <label><input type="radio" name="authorized" value="yes" required> Yes</label>
   <label><input type="radio" name="authorized" value="no"> No</label></fieldset>
 <div class="field"><label for="why">Why do you want to work at Acme?</label><textarea id="why" name="why"></textarea></div>
 <div class="field"><label for="cover">Cover letter</label><textarea id="cover" name="cover_letter"></textarea></div>
 <div class="field"><label><input type="checkbox" name="agree" value="1" required> I agree to the privacy policy *</label></div>
 <input type="text" name="website_hp" style="position:absolute;left:-9999px" tabindex="-1" aria-hidden="true">
 <input type="hidden" name="token" value="t">
 <div class="grecaptcha-badge" style="position:fixed;right:0;bottom:0"><iframe title="recaptcha" src="https://www.google.com/recaptcha/enterprise/anchor?size=invisible" style="width:256px;height:60px;border:0"></iframe></div>
 <button type="submit" id="submit_app">Submit Application</button>
</form></body></html>"#;

fn two_step(ask: bool) -> String {
    let colour = if ask {
        r#"<label for="c">Favourite colour *</label><input id="c" name="colour" required>"#
    } else {
        ""
    };
    format!(
        r#"<!doctype html><html><head><title>Two steps</title></head><body>
<form id="f" method="post" action="/two-submit">
 <div id="s1"><h2>Contact</h2>
  <label for="n">Full name *</label><input id="n" name="n" required>
  <label for="e">Email *</label><input id="e" name="e" type="email" required>
  {colour}
  <button type="button" id="next">Next</button></div>
 <div id="s2" style="display:none"><h2>Questions</h2>
  <label for="w">Why us?</label><textarea id="w" name="w"></textarea>
  <button type="submit">Submit</button></div>
</form>
<script>document.getElementById('next').onclick=function(){{document.getElementById('s1').style.display='none';document.getElementById('s2').style.display='block';}}</script>
</body></html>"#
    )
}

const CAPTCHA: &str = r#"<!doctype html><html><body><form action="/submit" method="post">
 <label for="a">Name</label><input id="a" name="a"><label for="b">Email</label><input id="b" name="b" type="email">
 <iframe title="reCAPTCHA" src="https://www.google.com/recaptcha/api2/anchor?size=normal" style="width:304px;height:78px"></iframe>
 <button type="submit">Submit application</button></form></body></html>"#;

fn simple_form(action: &str, error: Option<&str>) -> String {
    let error = error.map(|e| format!(r#"<div role="alert">{e}</div>"#)).unwrap_or_default();
    format!(
        r#"<!doctype html><html><body>{error}<form id="application-form" action="{action}" method="post">
 <label for="a">Full name</label><input id="a" name="a"><label for="b">Email</label><input id="b" name="b" type="text">
 <button type="submit">Submit application</button></form></body></html>"#
    )
}

#[derive(Clone, Debug)]
struct Post {
    path: String,
    body: String,
}

struct Server {
    addr: SocketAddr,
    posts: Arc<Mutex<Vec<Post>>>,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn posts(&self) -> Vec<Post> {
        self.posts.lock().unwrap().clone()
    }
}

async fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let posts: Arc<Mutex<Vec<Post>>> = Arc::default();
    let log = posts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let log = log.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buf = [0u8; 8192];
                let (head_end, content_length) = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                while data.len() < head_end + content_length {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&data[..head_end]).to_string();
                let mut first = head.lines().next().unwrap_or("").split_whitespace();
                let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or("/"));
                let (path, query) = target.split_once('?').unwrap_or((target, ""));
                let body = String::from_utf8_lossy(&data[head_end..]).to_string();

                let html = |status: &str, body: &str| {
                    format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n{body}")
                };
                let see = |to: &str| format!("HTTP/1.1 303 See Other\r\nLocation: {to}\r\nConnection: close\r\n\r\n");
                let response = match (method, path) {
                    ("GET", "/gh") => html("200 OK", GREENHOUSE),
                    ("GET", "/two") => html("200 OK", &two_step(query.contains("ask=1"))),
                    ("GET", "/captcha") => html("200 OK", CAPTCHA),
                    ("GET", "/reject") => html("200 OK", &simple_form("/reject-post", None)),
                    ("GET", "/ambiguous") => html("200 OK", &simple_form("/ambiguous-post", None)),
                    ("GET", "/thanks") => html(
                        "200 OK",
                        "<h1>Thank you for applying!</h1><p>Your application has been submitted.</p>",
                    ),
                    ("GET", "/done") => html("200 OK", "<p>All set. We will be in touch soon.</p>"),
                    ("POST", "/reject-post") => {
                        log.lock().unwrap().push(Post { path: path.into(), body });
                        html("200 OK", &simple_form("/reject-post", Some("Email is not accepted for this job")))
                    }
                    ("POST", "/submit" | "/two-submit") => {
                        log.lock().unwrap().push(Post { path: path.into(), body });
                        see("/thanks")
                    }
                    ("POST", "/ambiguous-post") => {
                        log.lock().unwrap().push(Post { path: path.into(), body });
                        see("/done")
                    }
                    _ => html("404 Not Found", "not found"),
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Server { addr, posts }
}

/// A scripted model that answers by label from the field list it is shown, like a real one would.
struct LabelLlm {
    prompts: Mutex<Vec<(String, String)>>,
    /// Reply to "does this page confirm the application?".
    confirms: bool,
}

impl LabelLlm {
    fn new(confirms: bool) -> Self {
        Self { prompts: Mutex::default(), confirms }
    }
}

impl Llm for LabelLlm {
    fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value> {
        self.prompts.lock().unwrap().push((system.to_string(), user.to_string()));
        let reply = if schema["properties"].get("submitted").is_some() {
            json!({"submitted": self.confirms})
        } else {
            let list = user.split("<form_fields>").nth(1).and_then(|s| s.split("</form_fields>").next()).unwrap_or("[]");
            let fields: Vec<Value> = serde_json::from_str(list.trim()).unwrap_or_default();
            let mut cover = None;
            let answers: Vec<Value> = fields
                .iter()
                .map(|f| {
                    let id = f["id"].as_str().unwrap_or_default();
                    let label = f["label"].as_str().unwrap_or_default().to_lowercase();
                    let (value, source) = if label.contains("authorized") {
                        ("yes", "profile")
                    } else if label.contains("country") {
                        ("Vietnam", "profile")
                    } else if label.contains("cover letter") {
                        cover = Some("Dear Acme, I build backend systems in Rust and Go.");
                        ("", "generated")
                    } else if label.contains("why") {
                        ("I enjoy backend work in Rust.", "generated")
                    } else if label.contains("privacy") {
                        ("yes", "profile")
                    } else {
                        ("", "unknown")
                    };
                    json!({"id": id, "value": value, "source": source, "needs_input": value.is_empty() && cover.is_none()})
                })
                .collect();
            let mut reply = json!({"answers": answers});
            if let Some(c) = cover {
                reply["cover_letter"] = json!(c);
            }
            reply
        };
        Box::pin(std::future::ready(Ok(reply)))
    }

    fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }
}

fn setup(tmp: &Path, apply_url: &str) -> (Core, i64) {
    let core = Core::open_at(Paths::new(tmp.join("jp")), Secrets::memory()).unwrap();
    let profile = tmp.join("profile.yaml");
    std::fs::write(&profile, PROFILE).unwrap();
    core.set_profile_path(&profile).unwrap();
    let dir = core.paths.application_dir("acme-backend-engineer");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cv.yaml"), "summary:\n  - 4+ years of backend work in Rust and Go\n").unwrap();
    std::fs::write(dir.join("cv.pdf"), b"%PDF-1.4\n% fixture cv\n").unwrap();
    std::fs::write(dir.join("jd.md"), "# Backend Engineer\n\nWe need Rust.\n").unwrap();
    let app = core
        .store
        .create_application(&NewApplication {
            slug: "acme-backend-engineer".into(),
            company: "Acme".into(),
            role: "Backend Engineer".into(),
            apply_url: Some(apply_url.into()),
            dir,
            ..Default::default()
        })
        .unwrap();
    core.store.set_status(app.id, Status::CvReady, "test").unwrap();
    (core, app.id)
}

fn status(core: &Core, id: i64) -> Status {
    core.store.get_application(id).unwrap().unwrap().status
}

fn value_of(review: &flow::Review, label: &str) -> String {
    review.rows.iter().find(|r| r.label.starts_with(label)).unwrap_or_else(|| panic!("no row {label}: {:?}", review.rows)).value.clone()
}

fn review(prepared: Prepared) -> flow::Review {
    match prepared {
        Prepared::Review(r) => r,
        Prepared::Handoff(h) => panic!("unexpected handoff: {h:?}"),
    }
}

fn is_png(path: &Path) -> bool {
    std::fs::read(path).map(|b| b.starts_with(&[0x89, b'P', b'N', b'G'])).unwrap_or(false)
}

#[test]
#[ignore = "launches Chrome"]
fn apply_greenhouse_like_form_fills_reviews_and_submits() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;
        let (core, id) = setup(tmp.path(), &server.url("/gh"));
        let llm = LabelLlm::new(true);

        let stages = Arc::new(Mutex::new(Vec::new()));
        let seen = stages.clone();
        let prepared = flow::prepare_with(&core, id, ApplyOptions::default(), Some(&llm), move |s| seen.lock().unwrap().push(s)).await.unwrap();
        let r = review(prepared);

        // What was found and chosen.
        assert!(r.complete && !r.dry_run);
        assert_eq!(value_of(&r, "First Name"), "Vinh");
        assert_eq!(value_of(&r, "Last Name"), "Nguyen Phuc");
        assert_eq!(value_of(&r, "Email"), "vinh@example.com");
        assert_eq!(value_of(&r, "Country"), "Vietnam");
        assert_eq!(value_of(&r, "Are you authorized"), "Yes");
        assert!(value_of(&r, "Cover letter").starts_with("Dear Acme"));
        assert_eq!(r.rows.iter().find(|x| x.label == "Cover letter").unwrap().source, Source::Generated);
        assert!(r.rows.iter().all(|x| !x.label.contains("website_hp") && !x.label.is_empty()), "honeypot and unlabeled fields are skipped: {:?}", r.rows);
        assert_eq!(r.rows.iter().filter(|x| x.kind == FieldKind::File).count(), 1);
        assert_eq!(r.attachment.as_deref().and_then(|p| p.file_name()).and_then(|n| n.to_str()), Some("cv.pdf"));
        assert!(r.rows.iter().find(|x| x.label.starts_with("Country")).unwrap().options.contains(&"Vietnam".to_string()));
        assert!(r.rows.iter().find(|x| x.label.starts_with("Email")).unwrap().required);
        assert!(!r.rows.iter().find(|x| x.label.starts_with("Phone")).unwrap().required);
        assert!(r.screenshots.len() == 1 && is_png(&r.screenshots[0]), "{:?}", r.screenshots);
        assert!(stages.lock().unwrap().contains(&flow::Stage::Ready));

        // The page really holds those values, and nothing was sent yet.
        let page = test_page(&core, id).await.unwrap();
        let dom: Value = eval(
            &page,
            "(() => { const v = (n) => document.querySelector(`[name=${n}]`).value; return {first: v('first_name'), email: v('email'), phone: v('phone'), country: v('country'), why: v('why'), cover: v('cover_letter'), \
             authorized: document.querySelector('[name=authorized]:checked')?.value, agree: document.querySelector('[name=agree]').checked, file: document.querySelector('[name=resume]').files[0]?.name, hp: v('website_hp')}; })()".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(dom["first"], "Vinh");
        assert_eq!(dom["phone"], "0373000000");
        assert_eq!(dom["country"], "vn");
        assert_eq!(dom["authorized"], "yes");
        assert_eq!(dom["agree"], true);
        assert_eq!(dom["file"], "cv.pdf");
        assert_eq!(dom["hp"], "", "the honeypot is never filled");
        assert!(dom["cover"].as_str().unwrap().starts_with("Dear Acme"));
        assert!(server.posts().is_empty(), "review must not submit anything");
        assert_eq!(status(&core, id), Status::Submitting);
        assert!(matches!(flow::live(&core, id), flow::Live::Review(_)));

        // The prompt told the model that page data is untrusted and never showed it what the profile settled.
        let prompts = llm.prompts.lock().unwrap().clone();
        assert!(prompts[0].0.contains("UNTRUSTED"));
        assert!(!prompts[0].1.contains("\"Email\""), "profile-settled fields are not sent to the model");

        // The user edits one answer and confirms.
        let why_id = r.rows.iter().find(|x| x.label.starts_with("Why do you")).unwrap().id.clone();
        let edits = vec![Edit { id: why_id, value: "Edited by the user".into(), remember: true }];
        let done = flow::submit_with(&core, id, edits, Some(&llm)).await.unwrap();
        let Submitted::Applied { proof } = done else { panic!("expected Applied, got {done:?}") };
        assert!(is_png(&proof));

        let posts = server.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].path, "/submit");
        let body = &posts[0].body;
        for needle in [
            "name=\"first_name\"\r\n\r\nVinh",
            "name=\"email\"\r\n\r\nvinh@example.com",
            "name=\"country\"\r\n\r\nvn",
            "name=\"authorized\"\r\n\r\nyes",
            "name=\"why\"\r\n\r\nEdited by the user",
            "filename=\"cv.pdf\"",
            "% fixture cv",
        ] {
            assert!(body.contains(needle), "missing {needle:?} in {body}");
        }
        assert_eq!(status(&core, id), Status::Applied);
        assert!(matches!(flow::live(&core, id), flow::Live::Idle));
        assert_eq!(flow::latest_proof(&core, "acme-backend-engineer").as_deref(), Some(proof.as_path()));
        let events: Vec<String> = core.store.list_events(id).unwrap().into_iter().map(|e| e.message).collect();
        assert!(events.iter().any(|m| m.contains("submission confirmed")), "{events:?}");

        // "Remember" wrote the edited answer to the profile copy, and only there.
        let saved = super::read_answers(&core.profile_path()).unwrap();
        assert_eq!(saved, vec![("Why do you want to work at Acme?".to_string(), "Edited by the user".to_string())]);
        let profile_text = std::fs::read_to_string(core.profile_path()).unwrap();
        assert!(profile_text.starts_with(PROFILE), "the rest of the profile is untouched");
    });
}

#[test]
#[ignore = "launches Chrome"]
fn apply_dry_run_never_submits() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;
        let (core, id) = setup(tmp.path(), &server.url("/gh"));
        let llm = LabelLlm::new(true);

        // Option.
        let r = review(flow::prepare_with(&core, id, ApplyOptions { dry_run: true }, Some(&llm), |_| {}).await.unwrap());
        assert!(r.dry_run && r.complete);
        assert_eq!(value_of(&r, "Email"), "vinh@example.com");
        assert_eq!(status(&core, id), Status::CvReady, "a dry run does not move the status");
        assert!(matches!(flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.unwrap(), Submitted::DryRun));
        assert!(server.posts().is_empty());
        flow::discard(&core, id).await.unwrap();
        assert!(matches!(flow::live(&core, id), flow::Live::Idle));
        assert_eq!(status(&core, id), Status::CvReady);

        // JOBPILOT_DRY_RUN=1 forces it even when the option is off.
        std::env::set_var(super::DRY_RUN_ENV, "1");
        let r = review(flow::prepare_with(&core, id, ApplyOptions { dry_run: false }, Some(&llm), |_| {}).await.unwrap());
        assert!(r.dry_run, "the env var forces a dry run");
        assert!(matches!(flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.unwrap(), Submitted::DryRun));
        // Even if a run was prepared before the variable was set, submitting under it never clicks.
        std::env::remove_var(super::DRY_RUN_ENV);
        flow::discard(&core, id).await.unwrap();
        let r2 = review(flow::prepare_with(&core, id, ApplyOptions::default(), Some(&llm), |_| {}).await.unwrap());
        assert!(!r2.dry_run);
        std::env::set_var(super::DRY_RUN_ENV, "1");
        assert!(matches!(flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.unwrap(), Submitted::DryRun));
        std::env::remove_var(super::DRY_RUN_ENV);
        assert!(server.posts().is_empty(), "no run ever submitted");
        flow::discard(&core, id).await.unwrap();
    });
}

#[test]
#[ignore = "launches Chrome"]
fn apply_two_step_form_walks_both_steps() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;
        let (core, id) = setup(tmp.path(), &server.url("/two"));
        let llm = LabelLlm::new(true);

        let r = review(flow::prepare_with(&core, id, ApplyOptions::default(), Some(&llm), |_| {}).await.unwrap());
        assert!(r.complete, "the walk reached the step with the submit button");
        assert_eq!(r.screenshots.len(), 2, "one screenshot per step");
        assert_eq!(value_of(&r, "Full name"), "Vinh Nguyen Phuc");
        assert_eq!(value_of(&r, "Why us"), "I enjoy backend work in Rust.");
        let name = r.rows.iter().find(|x| x.label.starts_with("Full name")).unwrap();
        let why = r.rows.iter().find(|x| x.label.starts_with("Why us")).unwrap();
        assert_eq!((name.step, name.editable), (0, false), "earlier steps are read-only");
        assert_eq!((why.step, why.editable), (1, true));

        // Submit with the value of step 1 intact in the DOM.
        assert!(matches!(flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.unwrap(), Submitted::Applied { .. }));
        let posts = server.posts();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].body.contains("n=Vinh+Nguyen+Phuc") && posts[0].body.contains("w=I+enjoy+backend+work+in+Rust."), "{}", posts[0].body);
        assert_eq!(status(&core, id), Status::Applied);
    });
}

#[test]
#[ignore = "launches Chrome"]
fn apply_step_needing_input_waits_for_the_user() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;
        let (core, id) = setup(tmp.path(), &server.url("/two?ask=1"));
        let llm = LabelLlm::new(true);

        let r = review(flow::prepare_with(&core, id, ApplyOptions::default(), Some(&llm), |_| {}).await.unwrap());
        assert!(!r.complete, "step 1 has a required question nobody can answer");
        assert_eq!(r.missing_required(), 1);
        let colour = r.rows.iter().find(|x| x.label.starts_with("Favourite colour")).unwrap();
        assert!(colour.needs_input && colour.value.is_empty() && colour.editable);
        // Submitting early and continuing without the answer are both refused.
        assert!(flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.is_err());
        assert!(flow::advance_with(&core, id, Vec::new(), Some(&llm), |_| {}).await.is_err());
        assert!(server.posts().is_empty());

        let edits = vec![Edit { id: colour.id.clone(), value: "blue".into(), remember: false }];
        let r = review(flow::advance_with(&core, id, edits, Some(&llm), |_| {}).await.unwrap());
        assert!(r.complete);
        assert_eq!(value_of(&r, "Favourite colour"), "blue");
        assert!(r.rows.iter().any(|x| x.label.starts_with("Why us") && x.editable));
        assert!(matches!(flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.unwrap(), Submitted::Applied { .. }));
        assert!(server.posts()[0].body.contains("colour=blue"));
    });
}

#[test]
#[ignore = "launches Chrome"]
fn apply_captcha_hands_off_immediately() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;
        let (core, id) = setup(tmp.path(), &server.url("/captcha"));
        let llm = LabelLlm::new(true);

        let Prepared::Handoff(h) = flow::prepare_with(&core, id, ApplyOptions::default(), Some(&llm), |_| {}).await.unwrap() else {
            panic!("a visible CAPTCHA must hand off")
        };
        assert!(h.captcha, "{h:?}");
        assert!(h.screenshot.as_deref().is_some_and(is_png));
        assert_eq!(status(&core, id), Status::Failed);
        assert!(matches!(flow::live(&core, id), flow::Live::Handoff { headed: false, .. }));
        assert!(llm.prompts.lock().unwrap().is_empty(), "nothing was answered or filled");
        assert!(server.posts().is_empty());

        flow::finish_manually(&core, id, false).await.unwrap();
        assert!(matches!(flow::live(&core, id), flow::Live::Idle));
        assert_eq!(status(&core, id), Status::Failed);
    });
}

#[test]
#[ignore = "launches Chrome"]
fn apply_rejected_submission_becomes_a_handoff() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;
        let (core, id) = setup(tmp.path(), &server.url("/reject"));
        let llm = LabelLlm::new(true);

        let r = review(flow::prepare_with(&core, id, ApplyOptions::default(), Some(&llm), |_| {}).await.unwrap());
        assert_eq!(value_of(&r, "Email"), "vinh@example.com");
        let Submitted::Handoff(h) = flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.unwrap() else {
            panic!("a rejected form is a handoff")
        };
        assert!(h.reason.contains("Email is not accepted"), "{}", h.reason);
        assert_eq!(server.posts().len(), 1);
        assert_eq!(status(&core, id), Status::Failed);

        // The user can mark the result by hand afterwards.
        flow::finish_manually(&core, id, true).await.unwrap();
        assert_eq!(status(&core, id), Status::Applied);
    });
}

#[test]
#[ignore = "launches Chrome"]
fn apply_unclear_confirmation_is_decided_by_a_yes_no_question() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;

        // The model says the page confirms: Applied.
        let (core, id) = setup(tmp.path(), &server.url("/ambiguous"));
        let yes = LabelLlm::new(true);
        review(flow::prepare_with(&core, id, ApplyOptions::default(), Some(&yes), |_| {}).await.unwrap());
        assert!(matches!(flow::submit_with(&core, id, Vec::new(), Some(&yes)).await.unwrap(), Submitted::Applied { .. }));
        assert_eq!(status(&core, id), Status::Applied);
        let asked = yes.prompts.lock().unwrap().iter().filter(|(s, _)| s.contains("confirms")).count();
        assert_eq!(asked, 1);

        // No: handoff that warns the application may have gone through.
        let tmp2 = tempfile::tempdir().unwrap();
        let (core, id) = setup(tmp2.path(), &server.url("/ambiguous"));
        let no = LabelLlm::new(false);
        review(flow::prepare_with(&core, id, ApplyOptions::default(), Some(&no), |_| {}).await.unwrap());
        let Submitted::Handoff(h) = flow::submit_with(&core, id, Vec::new(), Some(&no)).await.unwrap() else { panic!("expected a handoff") };
        assert!(h.reason.contains("may have gone through"), "{}", h.reason);
        assert_eq!(status(&core, id), Status::Failed);
    });
}

#[test]
#[ignore = "launches Chrome"]
fn apply_without_a_model_fills_only_the_profile_and_blocks_on_required_fields() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let server = serve().await;
        let (core, id) = setup(tmp.path(), &server.url("/gh"));
        let r = review(flow::prepare_with(&core, id, ApplyOptions::default(), None, |_| {}).await.unwrap());
        assert_eq!(value_of(&r, "Email"), "vinh@example.com");
        assert!(r.notes.iter().any(|n| n.contains("No AI provider")));
        // Country and "authorized" (required) stay empty, so submitting is refused until the user answers.
        assert!(r.missing_required() >= 2, "{:?}", r.rows);
        let err = flow::submit_with(&core, id, Vec::new(), None).await.unwrap_err();
        assert!(format!("{err:#}").contains("required"), "{err:#}");
        assert!(server.posts().is_empty());
        flow::discard(&core, id).await.unwrap();
        assert_eq!(status(&core, id), Status::CvReady, "discarding goes back to CvReady");
    });
}

/// Dry run (fill, never submit) against real public postings to check that extraction works outside
/// the fixtures. URLs: `JOBPILOT_TEST_APPLY_URLS` (comma separated; defaults to Lever's demo company and
/// a Greenhouse board). Screenshots are copied to `JOBPILOT_TEST_SHOTS` when set. Needs network.
#[test]
#[ignore = "launches Chrome and needs network"]
fn apply_real_postings_dry_run() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let urls = std::env::var("JOBPILOT_TEST_APPLY_URLS").unwrap_or_else(|_| {
        "https://jobs.lever.co/leverdemo/58db3f8e-b108-47d1-9e6e-87a1712497cd,https://job-boards.greenhouse.io/greenhouse/jobs/8126988".into()
    });
    // Whatever happens, this test must never submit.
    std::env::set_var(super::DRY_RUN_ENV, "1");
    runtime::block_on(async {
        for url in urls.split(',').map(str::trim).filter(|u| !u.is_empty()) {
            let tmp = tempfile::tempdir().unwrap();
            let (core, id) = setup(tmp.path(), url);
            let llm = LabelLlm::new(false);
            let prepared =
                flow::prepare_with(&core, id, ApplyOptions { dry_run: true }, Some(&llm), |s| eprintln!("  {}", s.label())).await;
            eprintln!("== {url} ({})", super::detect_ats(url).label());
            match prepared {
                Ok(Prepared::Review(r)) => {
                    for row in &r.rows {
                        eprintln!(
                            "  [{}] {:<14} {}{} = {:?} ({}){}",
                            row.step,
                            row.kind.as_str(),
                            row.label,
                            if row.required { " *" } else { "" },
                            row.value.chars().take(60).collect::<String>(),
                            row.source.as_str(),
                            row.warning.as_deref().map(|w| format!(" !{w}")).unwrap_or_default()
                        );
                    }
                    eprintln!("  complete={} screenshots={:?}", r.complete, r.screenshots);
                    if let Ok(dir) = std::env::var("JOBPILOT_TEST_SHOTS") {
                        std::fs::create_dir_all(&dir).unwrap();
                        for (i, shot) in r.screenshots.iter().enumerate() {
                            let host = url.split('/').nth(2).unwrap_or("site");
                            std::fs::copy(shot, Path::new(&dir).join(format!("{host}-{i}.png"))).unwrap();
                        }
                    }
                    assert!(r.rows.len() >= 3, "expected a real form, got {:?}", r.rows);
                    assert!(matches!(flow::submit_with(&core, id, Vec::new(), Some(&llm)).await.unwrap(), Submitted::DryRun));
                }
                Ok(Prepared::Handoff(h)) => eprintln!("  handoff: {h:?}"),
                Err(e) => eprintln!("  error: {e:#}"),
            }
            flow::discard(&core, id).await.unwrap();
        }
    });
    std::env::remove_var(super::DRY_RUN_ENV);
}
