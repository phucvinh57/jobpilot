//! Application services of this context: thin `#[tauri::command]`s named `integrations_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.
//!
//! Secrets (an API key, the Gmail client secret) arrive as command arguments, go to the keyring
//! through `Core` and nowhere else: no DTO, event, log line or error message carries one. An error
//! message is scrubbed of every secret `Core` has read or written (`redact`) before it leaves.

use std::sync::Arc;

use anyhow::Result;
use jobpilot_core::llm;
use jobpilot_core::mail::{self, SECRET_CLIENT_SECRET};
use jobpilot_core::redact::redact;
use jobpilot_core::Core;
use tauri::ipc::Invoke;
use tauri::{AppHandle, State};

use super::download::{download_browser, Downloads};
use super::dto::{
    DownloadDto, GmailDto, ProviderDto, ProviderInput, ProvidersDto, SessionDto, SessionsDto,
};
use super::rules::{client_id as required_client_id, provider_from_form, typed_key};
use crate::shared::{run, run_sync, CommandResult};

type CoreState<'a> = State<'a, Arc<Core>>;

/// `result` with every known secret masked in its message.
fn scrub<T>(result: CommandResult<T>) -> CommandResult<T> {
    result.map_err(|mut e| {
        e.message = redact(&e.message).into_owned();
        e
    })
}

// ------------------------------------------------------------------ services

fn providers(core: &Core) -> Result<ProvidersDto> {
    Ok(ProvidersDto::new(&core.store.list_providers()?))
}

/// Whether a key is saved for provider `id`: a keyring that cannot be read says no (the form then
/// asks for a key, which is what is needed).
fn provider_has_key(core: &Core, id: i64) -> bool {
    match core.provider_api_key(id) {
        Ok(key) => key.is_some(),
        Err(e) => {
            tracing::warn!("reading whether provider {id} has a key failed: {e:#}");
            false
        }
    }
}

fn save_provider(core: &Core, input: &ProviderInput) -> Result<ProviderDto> {
    let provider = provider_from_form(
        input.id,
        &input.kind,
        &input.name,
        &input.base_url,
        &input.model,
        input.json_object,
    )?;
    let key = typed_key(input.api_key.clone());
    let saved = core.save_provider(&provider, key.as_deref())?;
    tracing::info!(provider = saved.id, "AI provider saved");
    Ok(ProviderDto::from(&saved))
}

/// The key to use for a form: the one typed, else the one saved for provider `id`.
fn key_for(core: &Core, id: i64, typed: Option<String>) -> Result<Option<String>> {
    match typed_key(typed) {
        Some(key) => Ok(Some(key)),
        None if id != 0 => core.provider_api_key(id),
        None => Ok(None),
    }
}

async fn test_provider(core: &Core, input: &ProviderInput) -> Result<()> {
    let provider = provider_from_form(
        input.id,
        &input.kind,
        &input.name,
        &input.base_url,
        &input.model,
        input.json_object,
    )?;
    let key = key_for(core, input.id, input.api_key.clone())?;
    let client = llm::from_provider(&provider, key);
    llm::test_llm(client.as_ref()).await
}

async fn list_models(core: &Core, input: &ProviderInput) -> Result<Vec<String>> {
    let provider = provider_from_form(
        input.id,
        &input.kind,
        &input.name,
        &input.base_url,
        &input.model,
        input.json_object,
    )?;
    let key = key_for(core, input.id, input.api_key.clone())?;
    llm::from_provider(&provider, key).list_models().await
}

fn sessions(core: &Core, download: DownloadDto) -> Result<SessionsDto> {
    let manager = core.sessions();
    let sessions = manager
        .list()?
        .iter()
        .map(|s| SessionDto::new(s, manager.login_in_progress(s.id)))
        .collect();
    Ok(SessionsDto {
        sessions,
        chrome: core.chrome().as_ref().map(Into::into),
        download,
    })
}

fn gmail(core: &Core) -> GmailDto {
    let has_secret = match core.secrets.get(SECRET_CLIENT_SECRET) {
        Ok(secret) => secret.is_some_and(|s| !s.trim().is_empty()),
        Err(e) => {
            tracing::warn!("reading whether the Gmail client secret is saved failed: {e:#}");
            false
        }
    };
    let minutes = core.gmail_poll_interval().as_secs() / 60;
    GmailDto::new(&core.gmail_state(), has_secret, minutes)
}

// ------------------------------------------------------------------ commands: providers

/// The saved AI providers and what each kind starts with.
#[tauri::command]
pub async fn integrations_providers(core: CoreState<'_>) -> CommandResult<ProvidersDto> {
    let core = core.inner().clone();
    run_sync(move || providers(&core)).await
}

/// Whether a key is saved for the provider (never the key).
#[tauri::command]
pub async fn integrations_provider_has_key(core: CoreState<'_>, id: i64) -> CommandResult<bool> {
    let core = core.inner().clone();
    run_sync(move || Ok(provider_has_key(&core, id))).await
}

/// Create (`id == 0`) or update a provider; a typed key goes to the keyring, an empty one keeps the saved key.
#[tauri::command]
pub async fn integrations_save_provider(
    core: CoreState<'_>,
    input: ProviderInput,
) -> CommandResult<ProviderDto> {
    let core = core.inner().clone();
    scrub(run_sync(move || save_provider(&core, &input)).await)
}

/// Ask the provider a tiny question; an error says why it does not work.
#[tauri::command]
pub async fn integrations_test_provider(
    core: CoreState<'_>,
    input: ProviderInput,
) -> CommandResult<()> {
    let core = core.inner().clone();
    scrub(run(async move { test_provider(&core, &input).await }).await)
}

/// The models the provider offers, for the picker.
#[tauri::command]
pub async fn integrations_list_models(
    core: CoreState<'_>,
    input: ProviderInput,
) -> CommandResult<Vec<String>> {
    let core = core.inner().clone();
    scrub(run(async move { list_models(&core, &input).await }).await)
}

/// Make the provider the default one (the one used for generation).
#[tauri::command]
pub async fn integrations_make_default(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.store.set_default_provider(id)).await
}

/// Delete the provider and its key.
#[tauri::command]
pub async fn integrations_remove_provider(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.remove_provider(id)).await
}

// ------------------------------------------------------------------ commands: sessions

/// The sessions, the browser they run on and the state of a browser download.
#[tauri::command]
pub async fn integrations_sessions(
    core: CoreState<'_>,
    downloads: State<'_, Downloads>,
) -> CommandResult<SessionsDto> {
    let core = core.inner().clone();
    let download = downloads.state();
    run_sync(move || sessions(&core, download)).await
}

/// Add a session and open its browser at the login address, so the user can log in.
#[tauri::command]
pub async fn integrations_create_session(
    core: CoreState<'_>,
    name: String,
    url: String,
) -> CommandResult<i64> {
    let manager = core.inner().sessions();
    run(async move {
        let session = manager.create(&name, &url).await?;
        manager.open_login(session.id).await?;
        tracing::info!(session = session.id, "session created, login window opened");
        Ok(session.id)
    })
    .await
}

/// Open the login window of a session again.
#[tauri::command]
pub async fn integrations_open_session(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.open_login(id).await.map(|_| ()) }).await
}

/// The user is done logging in: save the logins, close the window and check the session.
#[tauri::command]
pub async fn integrations_finish_login(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.finish_login(id).await.map(|_| ()) }).await
}

/// Check in the background whether the session is still logged in.
#[tauri::command]
pub async fn integrations_check_session(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.check(id).await.map(|_| ()) }).await
}

/// Forget the logins of a session (the session stays, as new).
#[tauri::command]
pub async fn integrations_sign_out_session(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.sign_out(id).await }).await
}

/// Delete a session and its browser profile.
#[tauri::command]
pub async fn integrations_delete_session(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.delete(id).await }).await
}

/// Download (or update) the managed browser. Resolves when done; progress goes out as
/// `integrations:browser-download` events.
#[tauri::command]
pub async fn integrations_download_browser(
    app: AppHandle,
    core: CoreState<'_>,
    downloads: State<'_, Downloads>,
) -> CommandResult<()> {
    download_browser(app, core.inner().clone(), downloads.inner()).await
}

// ------------------------------------------------------------------ commands: gmail

/// Gmail settings and sync status.
#[tauri::command]
pub async fn integrations_gmail(core: CoreState<'_>) -> CommandResult<GmailDto> {
    let core = core.inner().clone();
    run_sync(move || Ok(gmail(&core))).await
}

/// Save the OAuth client (the secret goes to the keyring; empty keeps the saved one) and sign in
/// through the system browser. Resolves with the account address, or fails when the user cancels
/// or the sign-in takes more than five minutes.
#[tauri::command]
pub async fn integrations_gmail_connect(
    core: CoreState<'_>,
    client_id: String,
    client_secret: Option<String>,
) -> CommandResult<String> {
    let core = core.inner().clone();
    scrub(
        run(async move {
            let id = required_client_id(&client_id)?;
            core.gmail_save_client(&id, client_secret.as_deref())?;
            let account = core.gmail_connect().await?;
            // The first sync starts right away; later ones every few minutes.
            mail::autostart(&core);
            Ok(account)
        })
        .await,
    )
}

/// Abort a sign-in that waits for the browser.
#[tauri::command]
pub async fn integrations_gmail_cancel_connect(core: CoreState<'_>) -> CommandResult<()> {
    core.inner().gmail_cancel_connect();
    Ok(())
}

/// Check the mailbox now. A failure is saved with the sync and shown by the next `integrations_gmail`.
#[tauri::command]
pub async fn integrations_gmail_sync(core: CoreState<'_>) -> CommandResult<()> {
    let core = core.inner().clone();
    run(async move {
        // The outcome (or error) is saved by the sync and shown on the card.
        let _ = mail::sync_once(&core).await;
        Ok(())
    })
    .await
}

/// Revoke the access and forget the account (the OAuth client stays).
#[tauri::command]
pub async fn integrations_gmail_disconnect(core: CoreState<'_>) -> CommandResult<()> {
    let core = core.inner().clone();
    run(async move { core.gmail_disconnect().await }).await
}

/// This context's commands; `main.rs` routes every `integrations_*` call here.
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        integrations_providers,
        integrations_provider_has_key,
        integrations_save_provider,
        integrations_test_provider,
        integrations_list_models,
        integrations_make_default,
        integrations_remove_provider,
        integrations_sessions,
        integrations_create_session,
        integrations_open_session,
        integrations_finish_login,
        integrations_check_session,
        integrations_sign_out_session,
        integrations_delete_session,
        integrations_download_browser,
        integrations_gmail,
        integrations_gmail_connect,
        integrations_gmail_cancel_connect,
        integrations_gmail_sync,
        integrations_gmail_disconnect
    ]
}

#[cfg(test)]
mod tests {
    use jobpilot_core::{Core, Paths, Secrets};

    use super::{gmail, provider_has_key, providers, save_provider, scrub, ProviderInput};
    use crate::shared::CommandError;

    const API_KEY: &str = "sk-TOPSECRET-api-key-1234";
    const CLIENT_SECRET: &str = "GOCSPX-TOPSECRET-client-5678";

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn input(id: i64, key: Option<&str>) -> ProviderInput {
        ProviderInput {
            id,
            kind: "openai".into(),
            name: "Local".into(),
            base_url: "http://localhost:11434/v1".into(),
            model: "llama3".into(),
            json_object: false,
            api_key: key.map(str::to_string),
        }
    }

    #[test]
    fn what_the_screen_reads_never_contains_a_saved_secret() {
        let (_dir, core) = core();
        let saved = save_provider(&core, &input(0, Some(API_KEY))).unwrap();
        core.gmail_save_client("abc.apps.googleusercontent.com", Some(CLIENT_SECRET))
            .unwrap();

        // The key is in the keyring...
        assert_eq!(
            core.provider_api_key(saved.id).unwrap().as_deref(),
            Some(API_KEY)
        );
        // ...and everything the web app can ask for says only that it is saved.
        assert!(provider_has_key(&core, saved.id));
        let everything = serde_json::json!({
            "saved": saved,
            "providers": providers(&core).unwrap(),
            "gmail": gmail(&core),
        })
        .to_string();
        for secret in [API_KEY, CLIENT_SECRET, "TOPSECRET"] {
            assert!(
                !everything.contains(secret),
                "{secret} leaked into: {everything}"
            );
        }
        assert!(everything.contains(r#""hasClientSecret":true"#));
    }

    #[test]
    fn an_empty_key_keeps_the_saved_one_and_a_new_provider_becomes_the_default() {
        let (_dir, core) = core();
        let first = save_provider(&core, &input(0, Some(API_KEY))).unwrap();
        assert!(first.is_default, "the first provider is the default");

        let edited = save_provider(
            &core,
            &ProviderInput {
                name: "Renamed".into(),
                ..input(first.id, None)
            },
        )
        .unwrap();
        assert_eq!(edited.name, "Renamed");
        assert_eq!(
            core.provider_api_key(first.id).unwrap().as_deref(),
            Some(API_KEY),
            "left empty: the key stays"
        );

        let replaced = save_provider(&core, &input(first.id, Some("  sk-new  "))).unwrap();
        assert_eq!(
            core.provider_api_key(replaced.id).unwrap().as_deref(),
            Some("sk-new")
        );

        assert!(
            save_provider(
                &core,
                &ProviderInput {
                    name: "  ".into(),
                    ..input(0, None)
                }
            )
            .is_err(),
            "a name is required"
        );
        assert!(
            !provider_has_key(&core, 999),
            "an unknown provider has no key"
        );
    }

    #[test]
    fn an_error_message_is_scrubbed_of_known_secrets() {
        let (_dir, core) = core();
        // Registers the secret with the redactor, like every write through `Core` does.
        core.secrets
            .set("test:key", "sk-LEAKY-secret-value-42")
            .unwrap();
        let failed: Result<(), CommandError> = Err(CommandError::new(
            "HTTP 401: invalid key sk-LEAKY-secret-value-42",
        ));
        let message = scrub(failed).unwrap_err().message;
        assert!(!message.contains("LEAKY"), "{message}");
        assert!(message.contains("HTTP 401"));
    }

    #[test]
    fn the_gmail_card_reads_without_a_client_or_an_account() {
        let (_dir, core) = core();
        let dto = gmail(&core);
        assert_eq!(
            (
                dto.client_id.as_str(),
                dto.has_client_secret,
                dto.account.is_none()
            ),
            ("", false, true)
        );
        assert_eq!(dto.poll_minutes, 10);
        assert!(dto.last_sync.is_none() && dto.last_report.is_none());
    }
}
