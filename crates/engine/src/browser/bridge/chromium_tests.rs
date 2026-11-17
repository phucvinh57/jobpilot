//! The bridge with a real Chromium and the built extension (`testkit`). `#[ignore]`d: run with
//! `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine bridge -- --ignored` after `bun run ext:build`.
//! Pages come from local fixture servers only.

use std::time::Duration;

use serde_json::json;

use super::testkit::{self, TestBrowser};
use super::BridgeError;
use crate::browser::guard::Policy;
use crate::browser::tab::{Clip, OpenOptions, Profile, Tab};
use crate::net::fixtures::Fixtures;

const LOAD: Duration = Duration::from_secs(20);

fn port(fx: &Fixtures) -> u16 {
    url::Url::parse(fx.base()).unwrap().port().unwrap()
}

fn signed() -> OpenOptions {
    OpenOptions { profile: Profile::Signed, focus: false, guard: None }
}

async fn open(browser: &TestBrowser, url: &str) -> Tab {
    let tab = browser.bridge.open(url, signed()).await.unwrap();
    tab.wait_loaded(LOAD).await;
    tab
}

#[tokio::test]
#[ignore = "launches Chromium with the extension"]
async fn bridge_reads_a_page() {
    let Some(browser) = testkit::launch().await else { return };
    let status = browser.bridge.status();
    assert!(status.connected && status.paired, "{status:?}");
    assert!(status.browser.as_deref().is_some_and(|b| b.starts_with("Chrome")), "{status:?}");

    let fx = Fixtures::start().await;
    fx.route("/page", 200, "text/html", "<!DOCTYPE html><html><head><title>Fixture page</title></head><body><h1 id=h>Jobs</h1></body></html>");
    let tab = open(&browser, &format!("{}/page", fx.base())).await;
    assert_eq!(tab.title().await.unwrap(), "Fixture page");
    assert_eq!(tab.url().await.unwrap(), format!("{}/page", fx.base()));
    let html = tab.content().await.unwrap();
    assert!(html.starts_with("<!DOCTYPE html>") && html.contains("<h1 id=\"h\">Jobs</h1>"), "{html}");
    assert_eq!(tab.eval("1 + 2").await.unwrap(), json!(3));
    assert_eq!(tab.eval("Promise.resolve('later')").await.unwrap(), json!("later"));
    assert_eq!(tab.eval_as::<String>("document.getElementById('h').textContent").await.unwrap(), "Jobs");
    assert_eq!(tab.eval("navigator.webdriver").await.unwrap(), json!(true), "automation is never hidden");
    let err = tab.eval("throw new Error('boom')").await.unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");

    fx.route("/second", 200, "text/html", "<title>Second</title>");
    tab.goto(&format!("{}/second", fx.base())).await.unwrap();
    tab.wait_loaded(LOAD).await;
    assert_eq!(tab.title().await.unwrap(), "Second");
    tab.close().await;
    assert!(!tab.is_open());
}

#[tokio::test]
#[ignore = "launches Chromium with the extension"]
async fn bridge_clicks_types_and_presses_keys() {
    let Some(browser) = testkit::launch().await else { return };
    let fx = Fixtures::start().await;
    fx.route(
        "/form",
        200,
        "text/html",
        "<title>Form</title><input id=name style='margin-top:1500px'>\
         <button id=go onclick=\"document.title = 'clicked:' + document.getElementById('name').value\">Go</button>\
         <script>window.keys = []; document.addEventListener('keydown', e => window.keys.push(e.key));</script>",
    );
    let tab = open(&browser, &format!("{}/form", fx.base())).await;
    tab.click("#name").await.unwrap();
    assert_eq!(tab.eval("document.activeElement.id").await.unwrap(), json!("name"), "the click focused the input below the fold");
    tab.type_text("Ada L").await.unwrap();
    tab.press_key("Backspace").await.unwrap();
    tab.press_key("Escape").await.unwrap();
    tab.press_key("ArrowDown").await.unwrap();
    assert_eq!(tab.eval("document.getElementById('name').value").await.unwrap(), json!("Ada "));
    let keys = tab.eval("window.keys").await.unwrap();
    assert_eq!(keys, json!(["A", "d", "a", " ", "L", "Backspace", "Escape", "ArrowDown"]));
    tab.click("#go").await.unwrap();
    assert_eq!(tab.title().await.unwrap(), "clicked:Ada", "document.title drops the trailing space");
    let missing = tab.click("#nope").await.unwrap_err();
    assert!(missing.to_string().contains("#nope"), "{missing}");
}

#[tokio::test]
#[ignore = "launches Chromium with the extension"]
async fn bridge_sets_files_and_takes_screenshots() {
    let Some(browser) = testkit::launch().await else { return };
    let fx = Fixtures::start().await;
    fx.route("/upload", 200, "text/html", "<title>Upload</title><input type=file id=cv><script>window.changed = 0; document.getElementById('cv').addEventListener('change', () => window.changed++);</script>");
    let tab = open(&browser, &format!("{}/upload", fx.base())).await;
    let dir = tempfile::tempdir().unwrap();
    let cv = dir.path().join("cv \"final\".pdf");
    std::fs::write(&cv, vec![7u8; 1234]).unwrap();
    tab.set_files("#cv", std::slice::from_ref(&cv)).await.unwrap();
    let picked = tab.eval("(() => { const f = document.getElementById('cv').files[0]; return [f.name, f.size, f.type, window.changed]; })()").await.unwrap();
    assert_eq!(picked, json!(["cv \"final\".pdf", 1234, "application/pdf", 1]));
    assert!(tab.set_files("input[name='nope']", &[cv]).await.is_err());

    let png = tab.screenshot(None).await.unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 100, "{} bytes", png.len());
    let clipped = tab.screenshot(Some(Clip { x: 0.0, y: 0.0, width: 200.0, height: 100.0, scale: 1.0 })).await.unwrap();
    assert!(clipped.starts_with(b"\x89PNG"));
}

#[tokio::test]
#[ignore = "launches Chromium with the extension"]
async fn bridge_reports_a_closed_tab_and_refuses_incognito_when_not_allowed() {
    let Some(browser) = testkit::launch().await else { return };
    let fx = Fixtures::start().await;
    fx.route("/", 200, "text/html", "<title>t</title>");
    let tab = open(&browser, &format!("{}/", fx.base())).await;
    let mut events = browser.bridge.subscribe();
    // Closed behind the tab's back: the extension's `tabClosed` event is what resolves `closed()`.
    browser.bridge.call("tab.close", json!({ "tab": tab.id() })).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), tab.closed()).await.expect("closed() resolves");
    assert!(!tab.is_open());
    let mut saw = false;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
        saw |= event == super::BridgeEvent::TabClosed(tab.id());
    }
    assert!(saw, "a TabClosed event");
    let err = tab.eval("1").await.unwrap_err();
    assert_eq!(err.downcast_ref::<BridgeError>(), Some(&BridgeError::NotOwned), "{err}");

    // An unpacked extension is not allowed in Incognito until the user says so.
    assert!(!browser.bridge.status().incognito_allowed);
    let err = browser.bridge.open(&format!("{}/", fx.base()), OpenOptions { profile: Profile::Private, focus: false, guard: None }).await.unwrap_err();
    assert_eq!(err.downcast_ref::<BridgeError>(), Some(&BridgeError::IncognitoNotAllowed), "{err}");
}

#[tokio::test]
#[ignore = "launches Chromium with the extension"]
async fn bridge_guard_keeps_a_page_off_hosts_the_policy_refuses() {
    let Some(browser) = testkit::launch().await else { return };
    let (public, private) = (Fixtures::start().await, Fixtures::start().await);
    let (a, b) = (port(&public), port(&private));
    // `*.test` resolves to 127.0.0.1 in the test browser; port `a` stands for a public host.
    public.route(
        "/page",
        200,
        "text/html",
        format!(
            "<title>Guarded</title><img src='http://other.test:{b}/pixel.png'>\
             <script>document.cookie = 'seen=1; path=/'; fetch('http://other.test:{b}/api').catch(() => {{}});</script>\
             <img src='http://jobs.test:{a}/ok.png'>"
        ),
    );
    public.route("/ok.png", 200, "image/png", "x");
    private.route("/pixel.png", 200, "image/png", "x").route("/api", 200, "text/plain", "secret");

    let opts = OpenOptions { profile: Profile::Signed, focus: false, guard: Some(Policy::OnlyPorts(vec![a])) };
    let tab = browser.bridge.open(&format!("http://jobs.test:{a}/page"), opts.clone()).await.unwrap();
    tab.wait_loaded(LOAD).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(tab.title().await.unwrap(), "Guarded");
    assert!(!public.hits_on("/ok.png").is_empty(), "allowed subresources load: {:?}", public.hits());
    assert!(private.hits().is_empty(), "the refused host saw {:?}", private.hits());
    assert!(tab.blocked() >= 2, "blocked {}", tab.blocked());

    // The cookie names stay in the browser; only the domain comes back.
    let domains = browser.bridge.cookie_domains("jobs.test", false).await.unwrap();
    assert_eq!(domains, ["jobs.test"]);

    // A refused address is refused before the browser asks for it.
    assert!(tab.goto(&format!("http://other.test:{b}/api")).await.is_err());
    // An address literal on this machine is blocked by the extension's own rule.
    assert!(tab.goto(&format!("http://127.0.0.1:{a}/page")).await.is_err());
    assert!(public.hits_on("/page").len() == 1, "{:?}", public.hits());
    assert!(private.hits().is_empty());
    assert!(browser.bridge.open(&format!("http://other.test:{b}/"), opts).await.is_err());
}
