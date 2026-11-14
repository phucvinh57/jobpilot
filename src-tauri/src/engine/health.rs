//! What the app needs from the machine, and how to tell the user what is missing.
//!
//! - [`Health`]: which prerequisites are in place (an AI provider, the `typst` binary, Chrome).
//! - [`classify`]: turns an error message from any pipeline stage into a [`Problem`], so the UI can
//!   offer the right fix (open Settings, show the install command, retry) instead of a bare error.

use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::engine::pipeline::render::typst_binary;
use crate::engine::Core;

/// The command that installs the Typst CLI.
pub const TYPST_INSTALL_HINT: &str = "cargo install --locked typst-cli";

/// Why a site cannot be used without the user: something only they may do in the visible browser.
/// The app never solves or evades these (see `docs/plans/README.md`, rules).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Challenge {
    /// The site wants a login.
    Login,
    /// The site shows a captcha.
    Captcha,
    /// The site refuses automated access (403/429, block page).
    Blocked,
}

impl Challenge {
    /// `login` / `captcha` / `blocked`.
    pub fn as_str(self) -> &'static str {
        match self {
            Challenge::Login => "login",
            Challenge::Captcha => "captcha",
            Challenge::Blocked => "blocked",
        }
    }
}

/// Why an operation failed, in terms the user can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// No default AI provider is configured.
    NoProvider,
    /// The `typst` binary is missing.
    NoTypst,
    /// No Chrome/Chromium to drive.
    NoChrome,
    /// The network (or the remote server) did not answer: trying again may help.
    Network,
    /// A site waits for the user (login, captcha, blocked): open the browser, do it, continue.
    /// Which source and page it is travels with the paused run, not with the problem.
    NeedsHuman(Challenge),
    Other,
}

impl Problem {
    /// The Settings section that fixes this problem, if there is one.
    pub fn settings_section(self) -> Option<&'static str> {
        match self {
            Problem::NoProvider => Some("AI providers"),
            Problem::NoChrome => Some("Browser sessions"),
            _ => None,
        }
    }

    /// One line telling the user what to do.
    pub fn advice(self) -> Option<String> {
        match self {
            Problem::NoProvider => Some("Add an AI provider in Settings > AI providers and make it the default.".into()),
            Problem::NoChrome => {
                Some("Install Chrome or Chromium, or download a browser in Settings > Browser sessions.".into())
            }
            Problem::NoTypst => Some(format!(
                "Install Typst with `{TYPST_INSTALL_HINT}` (or set JOBPILOT_TYPST to the binary), then try again."
            )),
            Problem::Network => Some("The connection failed. Check your network and try again.".into()),
            Problem::NeedsHuman(Challenge::Login) => {
                Some("This site needs you to sign in. Open the browser, log in, then continue.".into())
            }
            Problem::NeedsHuman(Challenge::Captcha) => {
                Some("This site shows a captcha. Open the browser, solve it yourself, then continue.".into())
            }
            Problem::NeedsHuman(Challenge::Blocked) => Some(
                "This site is blocking automated access. Open the browser to look at the page, then continue or skip this source."
                    .into(),
            ),
            Problem::Other => None,
        }
    }

    /// Trying the same thing again can succeed without changing anything.
    pub fn retryable(self) -> bool {
        matches!(self, Problem::Network)
    }
}

/// Classify an error message (`format!("{e:#}")`).
pub fn classify(message: &str) -> Problem {
    let m = message.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    if has(&["no ai provider"]) {
        Problem::NoProvider
    } else if has(&["typst was not found"]) {
        Problem::NoTypst
    } else if has(&["chrome not found", "no chrome found"]) {
        Problem::NoChrome
    } else if has(&[
        "cannot connect",
        "timed out",
        "network error",
        "dns error",
        "error sending request",
        "connection refused",
        "connection reset",
        "connection closed",
        "could not resolve",
        "failed to lookup address",
        "temporary failure in name resolution",
        "tcp connect error",
        "http 502",
        "http 503",
        "http 504",
        "http 429",
    ]) {
        Problem::Network
    } else {
        Problem::Other
    }
}

/// Snapshot of the prerequisites.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Health {
    /// `typst --version` output, or `None` when the binary cannot be run.
    pub typst: Option<String>,
    /// A Chrome/Chromium is installed (or downloaded).
    pub chrome: bool,
    /// A default AI provider exists.
    pub provider: bool,
}

impl Health {
    /// Problems worth showing on every screen: what the main flow (JD -> CV) cannot run without.
    pub fn blockers(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        if !self.provider {
            out.push(Problem::NoProvider);
        }
        if self.typst.is_none() {
            out.push(Problem::NoTypst);
        }
        out
    }
}

/// Run `typst --version`; `None` when it is missing or broken.
pub async fn typst_version() -> Option<String> {
    let mut cmd = Command::new(typst_binary());
    cmd.arg("--version").kill_on_drop(true).stdin(std::process::Stdio::null());
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Core {
    /// Check the prerequisites (spawns `typst --version`, so run it on the core runtime).
    pub async fn health(&self) -> Health {
        let provider = matches!(self.store.default_provider(), Ok(Some(_)));
        Health { typst: typst_version().await, chrome: self.chrome().is_some(), provider }
    }

    /// Where `typst` was found (for the settings/diagnostics line).
    pub fn typst_path(&self) -> PathBuf {
        typst_binary()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_classified_by_what_the_user_can_do() {
        let cases = [
            ("no AI provider is set up: add one in Settings > AI providers", Problem::NoProvider),
            ("typst was not found (typst): install it", Problem::NoTypst),
            ("Chrome not found: install Chrome/Chromium", Problem::NoChrome),
            ("No Chrome found: install Chrome/Chromium or download the browser in Settings.", Problem::NoChrome),
            ("OpenAI: cannot connect (tcp connect error: Connection refused)", Problem::Network),
            ("OpenAI: the request timed out", Problem::Network),
            ("fetching https://x.test: error sending request for url", Problem::Network),
            ("OpenAI: HTTP 503 Service Unavailable", Problem::Network),
            ("OpenAI: HTTP 401 Unauthorized: invalid api key", Problem::Other),
            ("typst failed:\nerror: unknown variable", Problem::Other),
        ];
        for (message, expected) in cases {
            assert_eq!(classify(message), expected, "{message}");
        }
        assert!(Problem::Network.retryable() && !Problem::NoProvider.retryable());
        assert!(Problem::NoTypst.advice().unwrap().contains("cargo install --locked typst-cli"));
        assert_eq!(Problem::NoChrome.settings_section(), Some("Browser sessions"));
    }

    #[test]
    fn needs_human_asks_the_user_to_act_in_the_browser() {
        for challenge in [Challenge::Login, Challenge::Captcha, Challenge::Blocked] {
            let problem = Problem::NeedsHuman(challenge);
            let advice = problem.advice().unwrap();
            assert!(advice.contains("browser"), "{advice}");
            assert!(!problem.retryable(), "trying again without the user changes nothing");
            assert_eq!(problem.settings_section(), None);
        }
        assert!(Problem::NeedsHuman(Challenge::Captcha).advice().unwrap().contains("captcha"));
        assert_eq!(Challenge::Blocked.as_str(), "blocked");
        // Error text never classifies as NeedsHuman: only a paused run says so.
        assert_eq!(classify("the site shows a captcha"), Problem::Other);
    }

    #[test]
    fn blockers_lists_what_the_main_flow_needs() {
        assert_eq!(Health::default().blockers(), [Problem::NoProvider, Problem::NoTypst]);
        let ok = Health { typst: Some("typst 0.13".into()), chrome: false, provider: true };
        assert!(ok.blockers().is_empty(), "Chrome is only needed to apply");
    }
}
