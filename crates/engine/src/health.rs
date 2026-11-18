//! What the app needs from the machine, and how to tell the user what is missing.
//!
//! - [`Health`]: which prerequisites are in place (an AI provider, the `typst` binary, the browser
//!   extension).
//! - [`classify`]: turns an error message from any pipeline stage into a [`Problem`], so the UI can
//!   offer the right fix (open Settings, show the install command, retry) instead of a bare error.

use std::path::PathBuf;
use std::time::Duration;

use crate::browser::{INCOGNITO_NOT_ALLOWED, NOT_CONNECTED};
use crate::llm::codex::SIGN_IN_ERRORS;
use crate::pipeline::render::{typst_binary, typst_command};
use crate::Core;

/// The command that installs the Typst CLI.
#[cfg(windows)]
pub const TYPST_INSTALL_HINT: &str = "winget install --id Typst.Typst";
#[cfg(not(windows))]
pub const TYPST_INSTALL_HINT: &str = "cargo install --locked typst-cli";

/// Why a site cannot be used without the user: something only they may do in the visible browser.
/// The app never solves or evades these.
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
    /// The AI provider needs the user to sign in (again): a Codex provider that is not signed in,
    /// whose ChatGPT sign-in expired, or whose linked Codex CLI login is gone.
    ProviderSignIn,
    /// The user has not told JobPilot about their work yet (no documents read into a profile).
    NoProfile,
    /// The `typst` binary is missing.
    NoTypst,
    /// The JobPilot extension in the user's Chrome is not connected (`incognito: false`), or it may
    /// not open private tabs because it is not allowed in Incognito (`incognito: true`).
    NoBrowser { incognito: bool },
    /// The network (or the remote server) did not answer: trying again may help.
    Network,
    /// A site waits for the user (login, captcha, blocked): open the browser, do it, continue.
    /// Which source and page it is travels with the paused run, not with the problem.
    NeedsHuman(Challenge),
    Other,
}

impl Problem {
    /// The section that fixes this problem, if there is one: a Settings section, or "About you"
    /// (the web app shows that one on the Profile page).
    pub fn settings_section(self) -> Option<&'static str> {
        match self {
            Problem::NoProvider | Problem::ProviderSignIn => Some("AI providers"),
            Problem::NoProfile => Some("About you"),
            Problem::NoBrowser { .. } => Some("Browser"),
            _ => None,
        }
    }

    /// The label of the button that opens [`settings_section`](Self::settings_section), when it is
    /// more than "Open Settings > <section>".
    pub fn call_to_action(self) -> Option<&'static str> {
        match self {
            Problem::NoBrowser { incognito: false } => Some("Connect the JobPilot extension"),
            _ => None,
        }
    }

    /// One line telling the user what to do.
    pub fn advice(self) -> Option<String> {
        match self {
            Problem::NoProvider => Some("Add an AI provider in Settings > AI providers and make it the default.".into()),
            Problem::ProviderSignIn => Some(
                "Open the provider in Settings > AI providers and sign in with ChatGPT (or use your Codex CLI login) again."
                    .into(),
            ),
            Problem::NoProfile => Some("Add your CV or describe your work in Profile > About you.".into()),
            Problem::NoBrowser { incognito: false } => Some(
                "Connect the JobPilot extension: open Chrome with the extension and pair it in Settings > Browser.".into(),
            ),
            Problem::NoBrowser { incognito: true } => Some(
                "Allow the JobPilot extension in Incognito: on chrome://extensions open JobPilot > Details and turn on \"Allow in Incognito\"."
                    .into(),
            ),
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
    } else if SIGN_IN_ERRORS.iter().any(|e| m.contains(&e.to_ascii_lowercase())) {
        Problem::ProviderSignIn
    } else if has(&["no profile yet"]) {
        Problem::NoProfile
    } else if has(&["typst was not found"]) {
        Problem::NoTypst
    } else if m.contains(&INCOGNITO_NOT_ALLOWED.to_ascii_lowercase()) {
        Problem::NoBrowser { incognito: true }
    } else if m.contains(&NOT_CONNECTED.to_ascii_lowercase()) {
        Problem::NoBrowser { incognito: false }
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
    /// The JobPilot extension is connected (paired).
    pub browser: bool,
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
    let mut cmd = typst_command(&typst_binary());
    cmd.arg("--version");
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Core {
    /// Check the prerequisites (spawns `typst --version`, so run it on the core runtime).
    pub async fn health(&self) -> Health {
        let provider = matches!(self.store.default_provider(), Ok(Some(_)));
        Health { typst: typst_version().await, browser: self.browser().status().connected, provider }
    }

    /// Where `typst` was found (for the settings/diagnostics line).
    pub fn typst_path(&self) -> PathBuf {
        typst_binary()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::BridgeError;

    #[test]
    fn errors_are_classified_by_what_the_user_can_do() {
        let cases = [
            ("no AI provider is set up: add one in Settings > AI providers", Problem::NoProvider),
            ("typst was not found (typst): install it", Problem::NoTypst),
            ("OpenAI: cannot connect (tcp connect error: Connection refused)", Problem::Network),
            ("OpenAI: the request timed out", Problem::Network),
            ("fetching https://x.test: error sending request for url", Problem::Network),
            ("OpenAI: HTTP 503 Service Unavailable", Problem::Network),
            ("OpenAI: HTTP 401 Unauthorized: invalid api key", Problem::Other),
            ("typst failed:\nerror: unknown variable", Problem::Other),
            (crate::profile::NO_PROFILE, Problem::NoProfile),
        ];
        for (message, expected) in cases {
            assert_eq!(classify(message), expected, "{message}");
        }
        let not_connected = format!("{:#}", anyhow::Error::new(BridgeError::NotConnected).context("opening the start page"));
        assert_eq!(classify(&not_connected), Problem::NoBrowser { incognito: false }, "{not_connected}");
        let incognito = BridgeError::IncognitoNotAllowed.to_string();
        assert_eq!(classify(&incognito), Problem::NoBrowser { incognito: true }, "{incognito}");
        assert!(Problem::Network.retryable() && !Problem::NoProvider.retryable());
        assert!(Problem::NoTypst.advice().unwrap().contains(TYPST_INSTALL_HINT));
        assert_eq!(Problem::NoBrowser { incognito: false }.settings_section(), Some("Browser"));
        assert!(Problem::NoBrowser { incognito: false }.advice().unwrap().contains("Connect the JobPilot extension"));
        assert!(Problem::NoBrowser { incognito: true }.advice().unwrap().contains("Allow in Incognito"));
        assert_eq!(Problem::NoBrowser { incognito: false }.call_to_action(), Some("Connect the JobPilot extension"));
        assert_eq!(Problem::NoBrowser { incognito: true }.call_to_action(), None);
        assert_eq!(Problem::NoProvider.call_to_action(), None);
        // The web app opens "About you" on the Profile page; the message and advice say so too.
        assert_eq!(Problem::NoProfile.settings_section(), Some("About you"));
        assert!(Problem::NoProfile.advice().unwrap().contains("Profile > About you"));
        assert!(crate::profile::NO_PROFILE.contains("Profile > About you"));
    }

    #[test]
    fn a_codex_sign_in_error_asks_to_sign_in_again() {
        use crate::llm::codex::{CLI_LOGIN_GONE, NOT_SIGNED_IN, SIGN_IN_INVALID};
        for message in [
            format!("Codex: {NOT_SIGNED_IN}"),
            format!("Codex: {SIGN_IN_INVALID} (HTTP 400 Bad Request: invalid_grant): press Sign in with ChatGPT"),
            format!("Codex: {CLI_LOGIN_GONE}"),
            format!("calling the provider: Codex: {}", CLI_LOGIN_GONE.to_uppercase()),
        ] {
            assert_eq!(classify(&message), Problem::ProviderSignIn, "{message}");
        }
        let problem = Problem::ProviderSignIn;
        assert_eq!(problem.settings_section(), Some("AI providers"));
        assert_eq!(problem.call_to_action(), None, "the button says \"Open Settings > AI providers\"");
        assert!(problem.advice().is_some_and(|a| a.contains("sign in")));
        assert!(!problem.retryable(), "nothing changes until the user signs in");
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
        let ok = Health { typst: Some("typst 0.13".into()), browser: false, provider: true };
        assert!(ok.blockers().is_empty(), "the browser is only needed to apply and for some sources");
    }
}
