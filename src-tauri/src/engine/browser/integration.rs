//! Browser integration tests: they start a real Chrome, so they are `#[ignore]`d by default.
//! Run with `cargo test -p jobpilot -- --ignored browser` (set `JOBPILOT_CHROME` /
//! `JOBPILOT_NO_SANDBOX=1` when the system Chrome is elsewhere / cannot sandbox).

use std::net::SocketAddr;

use chromiumoxide::cdp::browser_protocol::network::CookieParam;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::SessionStatus;
use crate::engine::{runtime, Core, Paths, Secrets};

fn core_in(dir: &std::path::Path) -> Core {
    Core::open_at(Paths::new(dir.join("jp")), Secrets::memory()).unwrap()
}

/// Tiny HTTP server: `/` sets a persistent cookie, `/login` shows a password field, `/go` redirects to it.
async fn serve() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let cookie = "Set-Cookie: sid=abc123; Path=/; Max-Age=3600\r\n";
                let response = match path.as_str() {
                    "/login" => format!(
                        "HTTP/1.1 200 OK\r\n{cookie}Content-Type: text/html\r\nConnection: close\r\n\r\n\
                         <form><input name=u><input type=password name=p></form>"
                    ),
                    "/go" => format!("HTTP/1.1 302 Found\r\n{cookie}Location: /login\r\nConnection: close\r\n\r\n"),
                    _ => format!(
                        "HTTP/1.1 200 OK\r\n{cookie}Content-Type: text/html\r\nConnection: close\r\n\r\n<h1>welcome back</h1>"
                    ),
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    addr
}

#[test]
#[ignore = "launches Chrome"]
fn browser_headless_reads_cookies_on_a_data_url() {
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let core = core_in(tmp.path());
        let mgr = core.sessions();
        let session = mgr.create("data", "https://example.test/").await.unwrap();

        let handle = mgr.launch(session.id, true).await.unwrap();
        assert!(handle.is_headless() && handle.is_alive().await);
        let page = handle.new_page("data:text/html,<p id=x>hello</p>").await.unwrap();
        let text: String = page.evaluate_expression("document.getElementById('x').innerText").await.unwrap().into_value().unwrap();
        assert_eq!(text, "hello");

        // A data: page cannot hold cookies itself, so set one for an explicit URL through CDP and read it back.
        let cookie = CookieParam::builder().name("k").value("v").url("https://example.test/").build().unwrap();
        page.set_cookie(cookie).await.unwrap();
        let cookies = handle.cookies().await.unwrap();
        assert!(cookies.iter().any(|c| c.name == "k" && c.value == "v"), "cookies: {cookies:?}");
        assert_eq!(handle.domains().await.unwrap(), vec!["example.test".to_string()]);

        // One live browser per profile.
        assert!(mgr.is_open(session.id));
        let second = mgr.launch(session.id, true).await;
        assert!(second.is_err(), "second launch on the same profile must fail");

        handle.close().await.unwrap();
        assert!(!mgr.is_open(session.id));
        // ...and the lock is free again once closed.
        mgr.launch(session.id, true).await.unwrap().close().await.unwrap();
    });
}

#[test]
#[ignore = "launches Chrome"]
fn browser_health_check_sign_out_and_delete() {
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let core = core_in(tmp.path());
        let mgr = core.sessions();
        let addr = serve().await;

        // Cookie for the start domain and no login page: ok.
        let ok = mgr.create("home", &format!("http://{addr}/")).await.unwrap();
        assert_eq!(mgr.get(ok.id).unwrap().session_status(), SessionStatus::New);
        assert_eq!(mgr.check(ok.id).await.unwrap(), SessionStatus::Ok);
        let row = mgr.get(ok.id).unwrap();
        assert_eq!(row.session_status(), SessionStatus::Ok);
        assert!(row.last_verified_at.is_some());
        assert_eq!(row.domains(), vec!["127.0.0.1".to_string()]);
        assert!(!mgr.is_open(ok.id), "check must release the profile");

        // Same cookie, but the page is a login form: expired.
        let login = mgr.create("login", &format!("http://{addr}/login")).await.unwrap();
        assert_eq!(mgr.check(login.id).await.unwrap(), SessionStatus::Expired);

        // Redirected to a login url: expired.
        let redirected = mgr.create("go", &format!("http://{addr}/go")).await.unwrap();
        assert_eq!(mgr.check(redirected.id).await.unwrap(), SessionStatus::Expired);

        // Sign out wipes the profile and resets the row; the next check sees a fresh profile.
        let dir = core.paths.session_dir(ok.id);
        mgr.sign_out(ok.id).await.unwrap();
        let row = mgr.get(ok.id).unwrap();
        assert_eq!(row.session_status(), SessionStatus::New);
        assert!(row.domains().is_empty() && row.last_verified_at.is_none());
        assert!(!dir.join("Default/Network/Cookies").exists());

        // Delete removes the row and the directory.
        mgr.delete(login.id).await.unwrap();
        assert!(mgr.get(login.id).is_err());
        assert!(!core.paths.session_dir(login.id).exists());
    });
}

/// Headed login flow on the real display: `Done` with the window still open, and `Done` after the user
/// closed the window themselves (simulated by closing every tab).
#[test]
#[ignore = "launches a visible Chrome window"]
fn browser_headed_login_flow() {
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let core = core_in(tmp.path());
        let mgr = core.sessions();
        let addr = serve().await;

        // Window still open when the user presses Done.
        let a = mgr.create("a", &format!("http://{addr}/")).await.unwrap();
        let handle = mgr.open_login(a.id).await.unwrap();
        assert!(!handle.is_headless() && handle.is_alive().await && mgr.login_in_progress(a.id));
        // Opening again while the window is up hands back the same browser instead of failing on the lock.
        mgr.open_login(a.id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert_eq!(mgr.finish_login(a.id).await.unwrap(), SessionStatus::Ok);
        assert!(!mgr.login_in_progress(a.id) && !mgr.is_open(a.id) && !handle.is_alive().await);
        assert_eq!(mgr.get(a.id).unwrap().domains(), vec!["127.0.0.1".to_string()]);

        // The user closed the window before pressing Done.
        let b = mgr.create("b", &format!("http://{addr}/")).await.unwrap();
        let handle = mgr.open_login(b.id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        for page in handle.pages().await.unwrap() {
            let _ = page.close().await;
        }
        for _ in 0..50 {
            if !handle.is_alive().await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(!handle.is_alive().await, "Chrome should exit when its last window closes");
        assert_eq!(mgr.finish_login(b.id).await.unwrap(), SessionStatus::Ok);
    });
}

/// Downloads ~170 MB of Chrome for Testing into a temp dir: also needs `JOBPILOT_TEST_DOWNLOAD=1`.
#[test]
#[ignore = "downloads Chrome for Testing"]
fn browser_download_managed_chrome() {
    if std::env::var_os("JOBPILOT_TEST_DOWNLOAD").is_none() {
        eprintln!("skipped: set JOBPILOT_TEST_DOWNLOAD=1 to download Chrome for Testing");
        return;
    }
    runtime::block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("jp"));
        let progress = std::sync::Arc::new(super::DownloadProgress::default());
        let exe = super::chrome::download_managed(&paths, Some(progress.clone())).await.unwrap();
        assert_eq!(progress.percent(), Some(100));
        assert_eq!(super::chrome::managed_executable(&paths), Some(exe.clone()));
        let out = std::process::Command::new(&exe).arg("--version").output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("Chrome"), "{out:?}");
        // Nothing left behind besides the version dir and the marker.
        let mut left: Vec<_> = std::fs::read_dir(super::chrome::managed_root(&paths))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), 2, "{left:?}");
    });
}
