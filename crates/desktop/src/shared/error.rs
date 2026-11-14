//! What a failed command tells the web app: the message and what the user can do about it.
//!
//! The classification (no AI provider, no Chrome, no typst, network, a site that needs the user)
//! lives in `jobpilot_core::health`; this file only carries it over the bridge, so every screen
//! shows the same advice, Settings shortcut and Retry.

use std::future::Future;

use jobpilot_core::health::{self, Problem};
use jobpilot_core::redact::redact;
use jobpilot_core::runtime;
use serde::Serialize;

/// A [`Problem`] as the web app reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemDto {
    /// `noProvider` / `noTypst` / `noChrome` / `network` / `needsHuman` / `other`.
    pub kind: &'static str,
    /// `login` / `captcha` / `blocked`, only with `needsHuman`.
    pub challenge: Option<&'static str>,
    /// One line telling the user what to do.
    pub advice: Option<String>,
    /// The Settings section that fixes it.
    pub settings_section: Option<&'static str>,
    /// Trying the same thing again can succeed.
    pub retryable: bool,
}

impl From<Problem> for ProblemDto {
    fn from(problem: Problem) -> Self {
        let (kind, challenge) = match problem {
            Problem::NoProvider => ("noProvider", None),
            Problem::NoTypst => ("noTypst", None),
            Problem::NoChrome => ("noChrome", None),
            Problem::Network => ("network", None),
            Problem::NeedsHuman(challenge) => ("needsHuman", Some(challenge.as_str())),
            Problem::Other => ("other", None),
        };
        Self {
            kind,
            challenge,
            advice: problem.advice(),
            settings_section: problem.settings_section(),
            retryable: problem.retryable(),
        }
    }
}

/// The error of every command: `{ problem, message }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub problem: ProblemDto,
    pub message: String,
}

impl CommandError {
    /// Classify `message` (the `{e:#}` text of an error). Every secret `Core` has read or written
    /// is masked first: no context can send one to the web app in an error.
    pub fn new(message: impl Into<String>) -> Self {
        let message = redact(&message.into()).into_owned();
        Self {
            problem: health::classify(&message).into(),
            message,
        }
    }

    /// An error with a known problem (a paused run that needs the user, for instance).
    pub fn with_problem(problem: Problem, message: impl Into<String>) -> Self {
        Self {
            problem: problem.into(),
            message: redact(&message.into()).into_owned(),
        }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(e: anyhow::Error) -> Self {
        Self::new(format!("{e:#}"))
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

/// Run core work on the core runtime and map its error. Every command goes through this (or
/// [`run_sync`]), so the bridge thread never does I/O.
pub async fn run<T, Fut>(fut: Fut) -> CommandResult<T>
where
    Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    match runtime::spawn(fut).await {
        Ok(result) => result.map_err(CommandError::from),
        Err(_) => Err(CommandError::new("the task was cancelled")),
    }
}

/// [`run`] for a blocking core call (a store read, a file write).
pub async fn run_sync<T, F>(work: F) -> CommandResult<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    run(async move { work() }).await
}

#[cfg(test)]
mod tests {
    use jobpilot_core::health::{Challenge, Problem};

    use super::{CommandError, ProblemDto};

    #[test]
    fn a_problem_carries_its_advice_and_actions() {
        let dto = ProblemDto::from(Problem::NoProvider);
        assert_eq!(dto.kind, "noProvider");
        assert_eq!(dto.settings_section, Some("AI providers"));
        assert!(dto.advice.is_some());
        assert!(!dto.retryable);

        let network = ProblemDto::from(Problem::Network);
        assert!(network.retryable, "network errors get a Retry");

        let human = ProblemDto::from(Problem::NeedsHuman(Challenge::Captcha));
        assert_eq!(
            (human.kind, human.challenge),
            ("needsHuman", Some("captcha"))
        );

        let other = ProblemDto::from(Problem::Other);
        assert_eq!((other.kind, other.advice), ("other", None));
    }

    #[test]
    fn an_error_is_classified_from_its_text() {
        let e = CommandError::from(
            anyhow::anyhow!("connection refused").context("calling the provider"),
        );
        assert_eq!(e.problem.kind, "network");
        assert_eq!(e.message, "calling the provider: connection refused");
        assert_eq!(CommandError::new("boom").problem.kind, "other");
    }

    #[test]
    fn errors_serialize_in_camel_case() {
        let json = serde_json::to_value(CommandError::new("no AI provider is configured"))
            .unwrap_or_default();
        assert_eq!(json["problem"]["kind"], "noProvider");
        assert_eq!(json["problem"]["settingsSection"], "AI providers");
    }
}
