//! What the app shell shows on every screen: the setup state (sidebar health line, setup banner),
//! the navigation badges, the appearance, and the one way a link leaves the app.

use std::sync::Arc;

use crate::engine::health::{Health, Problem};
use crate::engine::prefs::Appearance;
use crate::engine::Core;
use serde::Serialize;
use tauri::ipc::Invoke;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::error::{run, run_sync, CommandError, CommandResult, ProblemDto};
use super::insight::is_web_link;

// ------------------------------------------------------------------ rules

/// The one-line setup state at the bottom of the sidebar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthLine {
    /// Green when nothing is missing, amber when something needs a look.
    pub ok: bool,
    pub text: String,
    /// The section that helps with the first problem, if there is one (Settings, or Profile for
    /// "About you").
    pub focus: Option<&'static str>,
}

/// What to say about the prerequisites (AI provider, typst, the browser extension).
pub fn health_line(health: &Health) -> HealthLine {
    let mut problems = Vec::new();
    if !health.provider {
        problems.push(Problem::NoProvider);
    }
    if health.typst.is_none() {
        problems.push(Problem::NoTypst);
    }
    if !health.browser {
        problems.push(Problem::NoBrowser { incognito: false });
    }
    let focus = problems.iter().find_map(|p| p.settings_section());
    let text = match problems.as_slice() {
        [] => "All checks passing".to_string(),
        [Problem::NoProvider] => "No AI provider".to_string(),
        [Problem::NoTypst] => "typst is missing".to_string(),
        [Problem::NoBrowser { .. }] => "Browser extension not connected".to_string(),
        many => format!("{} setup problems", many.len()),
    };
    HealthLine {
        ok: problems.is_empty(),
        text,
        focus,
    }
}

/// A line of the setup banner: what is missing and how to fix it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub id: &'static str,
    pub title: &'static str,
    pub problem: ProblemDto,
}

/// The prerequisites the main flow (JD -> CV) cannot run without, as banner lines.
pub fn blockers(health: &Health) -> Vec<Blocker> {
    health
        .blockers()
        .into_iter()
        .filter_map(|problem| {
            let (id, title) = match problem {
                Problem::NoProvider => (
                    "setup-provider",
                    "No AI provider yet: JobPilot cannot write CVs without one.",
                ),
                Problem::NoTypst => (
                    "setup-typst",
                    "Typst is not installed: CVs cannot be rendered.",
                ),
                _ => return None,
            };
            Some(Blocker {
                id,
                title,
                problem: problem.into(),
            })
        })
        .collect()
}

// ------------------------------------------------------------------ read models

/// Everything the shell needs in one read.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellStatus {
    pub health: HealthLine,
    pub blockers: Vec<Blocker>,
    /// Job-search preferences were saved at least once (the banner points to Profile > Preferences
    /// until then).
    pub prefs_saved: bool,
    /// A profile was read from the user's documents (the banner points to Profile > About you
    /// until then).
    pub profile_ready: bool,
    /// New leads (the Jobs badge when nothing needs the user).
    pub new_leads: usize,
    /// How many applications exist.
    pub applications: usize,
}

// ------------------------------------------------------------------ commands

/// Setup state and navigation badges. Called at startup and after `shared:data-changed`.
#[tauri::command]
pub async fn shared_status(core: State<'_, Arc<Core>>) -> CommandResult<ShellStatus> {
    let core = core.inner().clone();
    run(async move {
        let health = core.health().await;
        Ok(ShellStatus {
            health: health_line(&health),
            blockers: blockers(&health),
            prefs_saved: core.store.has_preferences()?,
            profile_ready: core.store.stored_profile()?.is_some(),
            new_leads: core.new_lead_count()?,
            applications: core.store.list_applications()?.len(),
        })
    })
    .await
}

/// The saved appearance: `system` / `light` / `dark`.
#[tauri::command]
pub async fn shared_appearance(core: State<'_, Arc<Core>>) -> CommandResult<&'static str> {
    let core = core.inner().clone();
    run_sync(move || Ok(core.appearance()?.as_str())).await
}

#[tauri::command]
pub async fn shared_set_appearance(
    core: State<'_, Arc<Core>>,
    appearance: String,
) -> CommandResult<()> {
    let Some(choice) = Appearance::parse(&appearance) else {
        return Err(CommandError::new(format!(
            "unknown appearance: {appearance}"
        )));
    };
    let core = core.inner().clone();
    run_sync(move || core.set_appearance(choice)).await
}

/// Open an `http(s)` link in the system browser. The webview never navigates and the web app has no
/// permission to open anything itself: every link goes through here.
#[tauri::command]
pub async fn shared_open_url(app: AppHandle, url: String) -> CommandResult<()> {
    if !is_web_link(&url) {
        return Err(CommandError::new("only http(s) links can be opened"));
    }
    app.opener()
        .open_url(url.trim(), None::<&str>)
        .map_err(|e| CommandError::new(format!("opening the link failed: {e}")))
}

/// The shell's own commands (`shared_*`).
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        shared_status,
        shared_appearance,
        shared_set_appearance,
        shared_open_url
    ]
}

#[cfg(test)]
mod tests {
    use crate::engine::health::Health;

    use super::{blockers, health_line};

    fn health(provider: bool, typst: bool, browser: bool) -> Health {
        Health {
            typst: typst.then(|| "typst 0.13".to_string()),
            browser,
            provider,
        }
    }

    #[test]
    fn the_health_line_names_one_problem_and_counts_several() {
        let ok = health_line(&health(true, true, true));
        assert!(ok.ok);
        assert_eq!(ok.text, "All checks passing");
        assert_eq!(ok.focus, None);

        let no_provider = health_line(&health(false, true, true));
        assert!(!no_provider.ok);
        assert_eq!(no_provider.text, "No AI provider");
        assert_eq!(no_provider.focus, Some("AI providers"));

        let no_typst = health_line(&health(true, false, true));
        assert_eq!(no_typst.text, "typst is missing");
        assert_eq!(no_typst.focus, None, "no Settings section installs typst");

        let no_browser = health_line(&health(true, true, false));
        assert_eq!(no_browser.text, "Browser extension not connected");
        assert_eq!(no_browser.focus, Some("Browser"));
        assert_eq!(
            health_line(&health(false, false, false)).text,
            "3 setup problems"
        );
    }

    #[test]
    fn the_banner_lists_what_blocks_writing_a_cv() {
        assert!(
            blockers(&health(true, true, false)).is_empty(),
            "the browser is only needed to apply"
        );
        let all = blockers(&health(false, false, true));
        assert_eq!(
            all.iter().map(|b| b.id).collect::<Vec<_>>(),
            ["setup-provider", "setup-typst"]
        );
        assert!(all.iter().all(|b| b.problem.advice.is_some()));
    }
}
