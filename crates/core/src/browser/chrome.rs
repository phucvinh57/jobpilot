//! Locating (and, on explicit request, downloading) the Chrome used for managed sessions.
//!
//! Resolution order: `JOBPILOT_CHROME` -> app-managed Chrome-for-Testing under `<data_dir>/chrome/`
//! -> a system Chrome/Chromium. The download only ever happens when [`download_managed`] is called
//! (the "Download browser" button); nothing is fetched implicitly.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::Paths;

/// Env var that points at a Chrome/Chromium executable (highest priority).
pub const CHROME_ENV: &str = "JOBPILOT_CHROME";

/// Chrome-for-Testing release index (Stable channel with per-platform download URLs).
const CFT_INDEX_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeSource {
    /// `JOBPILOT_CHROME`.
    Env,
    /// Downloaded by the app into `<data_dir>/chrome/`.
    Managed,
    /// Found on the system (PATH or a standard install location).
    System,
}

impl ChromeSource {
    pub fn label(self) -> &'static str {
        match self {
            ChromeSource::Env => "JOBPILOT_CHROME",
            ChromeSource::Managed => "managed",
            ChromeSource::System => "system",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub path: PathBuf,
    pub source: ChromeSource,
}

/// `<data_dir>/chrome`.
pub fn managed_root(paths: &Paths) -> PathBuf {
    paths.data_dir.join("chrome")
}

/// The installed managed Chrome executable, if one was downloaded (`chrome/current` names the version).
pub fn managed_executable(paths: &Paths) -> Option<PathBuf> {
    let root = managed_root(paths);
    let version = std::fs::read_to_string(root.join("current")).ok()?;
    let exe = root.join(version.trim()).join(exe_relative_path(cft_platform()?));
    exe.is_file().then_some(exe)
}

/// Resolve the Chrome to launch, or `None` when nothing is installed.
pub fn resolve(paths: &Paths) -> Option<Chrome> {
    resolve_from(std::env::var_os(CHROME_ENV), managed_executable(paths), &system_candidates())
}

/// Pure resolution logic (order: env, managed, system); every candidate must be an existing file.
pub fn resolve_from(env: Option<OsString>, managed: Option<PathBuf>, system: &[PathBuf]) -> Option<Chrome> {
    if let Some(path) = env.filter(|v| !v.is_empty()).map(PathBuf::from) {
        if path.is_file() {
            return Some(Chrome { path, source: ChromeSource::Env });
        }
        tracing::warn!("{CHROME_ENV}={} does not exist; ignoring it", path.display());
    }
    if let Some(path) = managed.filter(|p| p.is_file()) {
        return Some(Chrome { path, source: ChromeSource::Managed });
    }
    system
        .iter()
        .find(|p| p.is_file())
        .map(|path| Chrome { path: path.clone(), source: ChromeSource::System })
}

/// Standard system browsers, most preferred first: PATH lookups, then well-known install paths.
fn system_candidates() -> Vec<PathBuf> {
    const NAMES: [&str; 6] =
        ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome", "chrome-browser"];
    let mut out: Vec<PathBuf> = NAMES.iter().filter_map(|n| find_in_path(n)).collect();
    let fixed: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
    } else if cfg!(windows) {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ]
    } else {
        &["/opt/google/chrome/chrome", "/snap/bin/chromium", "/usr/lib/chromium/chromium"]
    };
    out.extend(fixed.iter().map(PathBuf::from));
    out
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

// ---- download (Chrome for Testing) ----

/// Live download counters; poll from the UI while [`download_managed`] runs.
#[derive(Debug, Default)]
pub struct DownloadProgress {
    pub downloaded: AtomicU64,
    /// 0 while unknown.
    pub total: AtomicU64,
}

impl DownloadProgress {
    /// Percent done (0..=100) when the total is known.
    pub fn percent(&self) -> Option<u8> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.downloaded.load(Ordering::Relaxed) * 100 / total).min(100) as u8)
    }
}

/// Chrome-for-Testing platform id of this build, if it has a CfT download.
fn cft_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("mac-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("mac-x64")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("win64")
    } else {
        None
    }
}

/// Executable path inside an extracted CfT zip (relative to the version directory).
fn exe_relative_path(platform: &str) -> PathBuf {
    match platform {
        "linux64" => ["chrome-linux64", "chrome"].iter().collect(),
        "win64" | "win32" => [format!("chrome-{platform}"), "chrome.exe".to_string()].iter().collect(),
        mac => [
            format!("chrome-{mac}"),
            "Google Chrome for Testing.app".to_string(),
            "Contents".to_string(),
            "MacOS".to_string(),
            "Google Chrome for Testing".to_string(),
        ]
        .iter()
        .collect(),
    }
}

#[derive(Deserialize)]
struct CftIndex {
    channels: CftChannels,
}
#[derive(Deserialize)]
struct CftChannels {
    #[serde(rename = "Stable")]
    stable: CftChannel,
}
#[derive(Deserialize)]
struct CftChannel {
    version: String,
    downloads: CftDownloads,
}
#[derive(Deserialize)]
struct CftDownloads {
    #[serde(default)]
    chrome: Vec<CftDownload>,
}
#[derive(Deserialize)]
struct CftDownload {
    platform: String,
    url: String,
}

/// Download the Stable Chrome-for-Testing build for this platform into `<data_dir>/chrome/<version>/`
/// and make it the managed browser. Returns the executable path. Only call on explicit user action.
pub async fn download_managed(paths: &Paths, progress: Option<Arc<DownloadProgress>>) -> Result<PathBuf> {
    let platform = cft_platform().context("no Chrome-for-Testing build for this platform; set JOBPILOT_CHROME")?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()?;

    let index: CftIndex = client
        .get(CFT_INDEX_URL)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .context("fetching the Chrome-for-Testing index")?
        .json()
        .await
        .context("reading the Chrome-for-Testing index")?;
    let version = index.channels.stable.version;
    let url = index
        .channels
        .stable
        .downloads
        .chrome
        .into_iter()
        .find(|d| d.platform == platform)
        .map(|d| d.url)
        .with_context(|| format!("Chrome {version} has no {platform} download"))?;
    if !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        bail!("unexpected Chrome version {version:?}");
    }

    let root = managed_root(paths);
    let version_dir = root.join(&version);
    let exe = version_dir.join(exe_relative_path(platform));
    if !exe.is_file() {
        std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
        let zip_path = root.join(format!("{version}.zip.part"));
        download_to(&client, &url, &zip_path, progress.as_deref()).await?;

        let staging = root.join(format!("{version}.tmp"));
        let (zip, stage, dest) = (zip_path.clone(), staging.clone(), version_dir.clone());
        let extracted = tokio::task::spawn_blocking(move || -> Result<()> {
            let _ = std::fs::remove_dir_all(&stage);
            extract_zip(&zip, &stage)?;
            let _ = std::fs::remove_dir_all(&dest);
            std::fs::rename(&stage, &dest).context("installing the extracted browser")?;
            Ok(())
        })
        .await
        .context("extraction task failed")?;
        let _ = std::fs::remove_file(&zip_path);
        if extracted.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        extracted?;
    }
    if !exe.is_file() {
        bail!("downloaded archive does not contain {}", exe.display());
    }
    std::fs::write(root.join("current"), &version).context("recording the managed Chrome version")?;
    Ok(exe)
}

async fn download_to(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: Option<&DownloadProgress>,
) -> Result<()> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("downloading {url}"))?;
    if let Some(p) = progress {
        p.downloaded.store(0, Ordering::Relaxed);
        p.total.store(resp.content_length().unwrap_or(0), Ordering::Relaxed);
    }
    let mut file = std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    while let Some(chunk) = resp.chunk().await.context("download interrupted")? {
        file.write_all(&chunk).context("writing the download")?;
        if let Some(p) = progress {
            p.downloaded.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
    }
    file.flush()?;
    Ok(())
}

/// Extract a zip into `dest`, refusing paths that escape it and keeping unix permissions.
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?).context("opening the archive")?;
    std::fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            bail!("archive entry {:?} escapes the target directory", entry.name());
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            // symlink entry: the target is the entry body
            let mut target = String::new();
            std::io::Read::read_to_string(&mut entry, &mut target)?;
            std::os::unix::fs::symlink(target, &out)?;
            continue;
        }
        let mut file = std::fs::File::create(&out).with_context(|| format!("creating {}", out.display()))?;
        std::io::copy(&mut entry, &mut file)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "").unwrap();
        p
    }

    #[test]
    fn env_beats_managed_beats_system() {
        let dir = tempfile::tempdir().unwrap();
        let env = touch(dir.path(), "env-chrome");
        let managed = touch(dir.path(), "managed-chrome");
        let system = touch(dir.path(), "system-chrome");

        let all = resolve_from(Some(env.clone().into()), Some(managed.clone()), &[system.clone()]).unwrap();
        assert_eq!(all, Chrome { path: env, source: ChromeSource::Env });

        let no_env = resolve_from(None, Some(managed.clone()), &[system.clone()]).unwrap();
        assert_eq!(no_env, Chrome { path: managed, source: ChromeSource::Managed });

        let only_system = resolve_from(None, None, &[system.clone()]).unwrap();
        assert_eq!(only_system, Chrome { path: system, source: ChromeSource::System });
    }

    #[test]
    fn missing_candidates_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        let system = touch(dir.path(), "system-chrome");
        let gone = dir.path().join("does-not-exist");

        // env points nowhere, managed is gone: fall back to the first existing system candidate
        let got = resolve_from(Some(gone.clone().into()), Some(gone.clone()), &[gone.clone(), system.clone()]).unwrap();
        assert_eq!(got, Chrome { path: system, source: ChromeSource::System });

        // empty env var is ignored
        assert!(resolve_from(Some(OsString::new()), None, &[gone]).is_none());
        assert!(resolve_from(None, None, &[]).is_none());
    }

    #[test]
    fn managed_executable_follows_current_marker() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        assert!(managed_executable(&paths).is_none());

        let Some(platform) = cft_platform() else { return };
        let exe = managed_root(&paths).join("123.0.1").join(exe_relative_path(platform));
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        assert!(managed_executable(&paths).is_none(), "no `current` marker yet");

        std::fs::write(managed_root(&paths).join("current"), "123.0.1\n").unwrap();
        assert_eq!(managed_executable(&paths), Some(exe));
    }

    #[test]
    fn progress_percent() {
        let p = DownloadProgress::default();
        assert_eq!(p.percent(), None);
        p.total.store(200, Ordering::Relaxed);
        p.downloaded.store(50, Ordering::Relaxed);
        assert_eq!(p.percent(), Some(25));
    }

    #[test]
    fn cft_index_parses() {
        let json = r#"{"timestamp":"x","channels":{"Stable":{"channel":"Stable","version":"150.0.1.2","revision":"1",
            "downloads":{"chrome":[{"platform":"linux64","url":"https://x/chrome-linux64.zip"},
            {"platform":"win64","url":"https://x/chrome-win64.zip"}],"chromedriver":[]}}}}"#;
        let idx: CftIndex = serde_json::from_str(json).unwrap();
        assert_eq!(idx.channels.stable.version, "150.0.1.2");
        assert_eq!(idx.channels.stable.downloads.chrome.len(), 2);
    }
}
