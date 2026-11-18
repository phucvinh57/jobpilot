//! Application services of this context: thin `#[tauri::command]`s named `integrations_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.
//!
//! Secrets (an API key, the Gmail client secret) arrive as command arguments, go to the keyring
//! through `Core` and nowhere else: no DTO, event, log line or error message carries one. An error
//! message is scrubbed of every secret `Core` has read or written (`redact`) before it leaves.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use jobpilot_engine::llm::{self, Llm};
use jobpilot_engine::store::{Provider, ProviderKind};
use jobpilot_engine::mail::{self, SECRET_CLIENT_SECRET};
use jobpilot_engine::redact::redact;
use jobpilot_engine::Core;
use tauri::ipc::Invoke;
use tauri::{AppHandle, Manager, State};

use super::dto::{
    BrowserDto, BrowserSectionDto, GmailDto, ProviderDto, ProviderInput, ProvidersDto, SessionDto,
};
use super::rules::{self, client_id as required_client_id, provider_from_form, typed_key};
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

/// The ChatGPT account a Codex provider is signed in with (`None` for the other kinds, or when the
/// keyring cannot be read: the dialog then offers to sign in, which is what is needed).
fn codex_account(core: &Core, provider: &Provider) -> Option<String> {
    if provider.kind != ProviderKind::Codex {
        return None;
    }
    match core.codex_account(provider.id) {
        Ok(account) => account,
        Err(e) => {
            tracing::warn!(provider = provider.id, "reading the Codex sign-in failed: {e:#}");
            None
        }
    }
}

fn providers(core: &Core) -> Result<ProvidersDto> {
    Ok(ProvidersDto::new(&core.store.list_providers()?, |p| codex_account(core, p)))
}

/// Whether a key is saved for provider `id` (for Codex: whether it is signed in): a keyring that
/// cannot be read says no (the form then asks for a key, which is what is needed).
fn provider_has_key(core: &Core, id: i64) -> bool {
    let codex = match core.store.get_provider(id) {
        Ok(provider) => provider.filter(|p| p.kind == ProviderKind::Codex),
        Err(e) => {
            tracing::warn!("reading provider {id} failed: {e:#}");
            None
        }
    };
    if let Some(provider) = codex {
        return codex_account(core, &provider).is_some();
    }
    match core.provider_api_key(id) {
        Ok(key) => key.is_some(),
        Err(e) => {
            tracing::warn!("reading whether provider {id} has a key failed: {e:#}");
            false
        }
    }
}

/// The provider the form describes, and the stored row of a saved one (`None` for a new one).
fn provider_of(core: &Core, input: &ProviderInput) -> Result<(Provider, Option<Provider>)> {
    let saved = if input.id != 0 { core.store.get_provider(input.id)? } else { None };
    let provider = provider_from_form(input.id, &input.kind, &input.name, &input.base_url, &input.model, saved.as_ref())?;
    Ok((provider, saved))
}

fn save_provider(core: &Core, input: &ProviderInput) -> Result<ProviderDto> {
    let (provider, _) = provider_of(core, input)?;
    // A Codex provider signs in: a key typed for another kind before switching is not kept.
    let key = if rules::uses_key(provider.kind) { typed_key(input.api_key.clone()) } else { None };
    let saved = core.save_provider(&provider, key.as_deref())?;
    tracing::info!(provider = saved.id, "AI provider saved");
    Ok(ProviderDto::new(&saved, codex_account(core, &saved)))
}

/// The key to use for `form`: the one typed, else the one saved for that provider, but only while
/// the form still has the saved row's kind and base URL (a saved key never goes to another host).
fn key_for(core: &Core, form: &Provider, saved: Option<&Provider>, typed: Option<String>) -> Result<Option<String>> {
    match typed_key(typed) {
        Some(key) => Ok(Some(key)),
        None => match saved {
            Some(row) if llm::same_endpoint(row, form) => core.provider_api_key(row.id),
            _ => Ok(None),
        },
    }
}

/// The client for a form: a saved Codex provider uses its stored ChatGPT sign-in.
fn client_for(core: &Core, input: &ProviderInput) -> Result<Box<dyn Llm>> {
    let (provider, saved) = provider_of(core, input)?;
    let key = if rules::uses_key(provider.kind) {
        key_for(core, &provider, saved.as_ref(), input.api_key.clone())?
    } else {
        None
    };
    Ok(core.llm_for_form(&provider, key))
}

async fn test_provider(core: &Core, input: &ProviderInput) -> Result<()> {
    let client = client_for(core, input)?;
    llm::test_llm(client.as_ref()).await
}

async fn list_models(core: &Core, input: &ProviderInput) -> Result<Vec<String>> {
    client_for(core, input)?.list_models().await
}

/// The connection to the user's browser and the sites they are signed in to.
pub(super) fn browser_section(core: &Core, extension_path: Option<String>) -> Result<BrowserSectionDto> {
    let manager = core.sessions();
    let sessions = manager
        .list()?
        .iter()
        .map(|s| SessionDto::new(s, manager.login_in_progress(s.id)))
        .collect();
    Ok(BrowserSectionDto {
        browser: BrowserDto::new(&core.browser().status(), extension_path),
        sessions,
    })
}

/// Where the built extension is for "Load unpacked" (see [`rules::extension_path`]).
pub(super) fn extension_folder(app: &AppHandle) -> Option<String> {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../extension/dist");
    let checkout = checkout.canonicalize().unwrap_or(checkout);
    let bundled = app.path().resource_dir().ok().map(|dir| dir.join("extension"));
    rules::extension_path(&checkout, bundled).map(tauri::utils::display_path)
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

/// Sign the saved Codex provider in with ChatGPT in the system browser; resolves with the
/// account's email, or fails when the user cancels or the sign-in times out.
#[tauri::command]
pub async fn integrations_codex_sign_in(core: CoreState<'_>, id: i64) -> CommandResult<String> {
    let core = core.inner().clone();
    scrub(
        run(async move {
            let account = core.codex_sign_in(id).await?;
            tracing::info!(provider = id, "Codex provider signed in");
            Ok(account)
        })
        .await,
    )
}

/// Abort a Codex sign-in that waits for the browser.
#[tauri::command]
pub async fn integrations_codex_cancel_sign_in(core: CoreState<'_>) -> CommandResult<()> {
    core.inner().codex_cancel_sign_in();
    Ok(())
}

/// Reuse the Codex CLI's login for the saved Codex provider; resolves with the account's email.
#[tauri::command]
pub async fn integrations_codex_import_cli(core: CoreState<'_>, id: i64) -> CommandResult<String> {
    let core = core.inner().clone();
    scrub(
        run_sync(move || {
            let account = core.codex_import_cli_login(id)?;
            tracing::info!(provider = id, "Codex CLI login imported");
            Ok(account)
        })
        .await,
    )
}

/// Whether a Codex CLI login is there to reuse.
#[tauri::command]
pub async fn integrations_codex_cli_available() -> CommandResult<bool> {
    run_sync(|| Ok(llm::codex::cli_login_available())).await
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

// ------------------------------------------------------------------ commands: browser

/// The connection to the user's browser (the JobPilot extension) and the sites they are signed in to.
#[tauri::command]
pub async fn integrations_browser(app: AppHandle, core: CoreState<'_>) -> CommandResult<BrowserSectionDto> {
    let core = core.inner().clone();
    let path = extension_folder(&app);
    run_sync(move || browser_section(&core, path)).await
}

/// The user checked the code: pair the browser that waits.
#[tauri::command]
pub async fn integrations_approve_pairing(core: CoreState<'_>) -> CommandResult<()> {
    let bridge = core.inner().browser();
    run_sync(move || bridge.approve_pairing()).await
}

/// The user does not know the browser that asks to pair: refuse it.
#[tauri::command]
pub async fn integrations_decline_pairing(core: CoreState<'_>) -> CommandResult<()> {
    let bridge = core.inner().browser();
    run_sync(move || bridge.decline_pairing()).await
}

/// Forget the paired browser: it has to pair again (the user stays signed in to their sites).
#[tauri::command]
pub async fn integrations_forget_browser(core: CoreState<'_>) -> CommandResult<()> {
    let bridge = core.inner().browser();
    run_sync(move || bridge.forget()).await
}

/// Add a site and open its sign-in page in a tab of the user's browser; resolves with its id.
#[tauri::command]
pub async fn integrations_create_session(
    core: CoreState<'_>,
    name: String,
    url: String,
) -> CommandResult<i64> {
    let manager = core.inner().sessions();
    run(async move {
        let session = manager.create(&name, &url)?;
        manager.open_login(session.id).await?;
        Ok(session.id)
    })
    .await
}

/// Open the sign-in tab of a site again (or bring it to the front).
#[tauri::command]
pub async fn integrations_open_session(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.open_login(id).await }).await
}

/// The user is done signing in: close the sign-in tab and check the site.
#[tauri::command]
pub async fn integrations_finish_login(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.finish_login(id).await.map(|_| ()) }).await
}

/// Check in a background tab whether the user is still signed in to the site.
#[tauri::command]
pub async fn integrations_check_session(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run(async move { manager.check(id).await.map(|_| ()) }).await
}

/// Remove a site from the list (the user stays signed in to it in their browser).
#[tauri::command]
pub async fn integrations_delete_session(core: CoreState<'_>, id: i64) -> CommandResult<()> {
    let manager = core.inner().sessions();
    run_sync(move || manager.delete(id)).await
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

/// This context's commands; `lib.rs` routes every `integrations_*` call here.
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        integrations_providers,
        integrations_provider_has_key,
        integrations_save_provider,
        integrations_test_provider,
        integrations_list_models,
        integrations_make_default,
        integrations_remove_provider,
        integrations_codex_sign_in,
        integrations_codex_cancel_sign_in,
        integrations_codex_import_cli,
        integrations_codex_cli_available,
        integrations_browser,
        integrations_approve_pairing,
        integrations_decline_pairing,
        integrations_forget_browser,
        integrations_create_session,
        integrations_open_session,
        integrations_finish_login,
        integrations_check_session,
        integrations_delete_session,
        integrations_gmail,
        integrations_gmail_connect,
        integrations_gmail_cancel_connect,
        integrations_gmail_sync,
        integrations_gmail_disconnect
    ]
}

#[cfg(test)]
mod tests {
    use jobpilot_engine::{Core, Paths, Secrets};

    use super::{
        browser_section, client_for, gmail, key_for, provider_has_key, provider_of, providers, save_provider,
        scrub, ProviderInput,
    };
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
            kind: "openai_compatible".into(),
            name: "Local".into(),
            base_url: "http://localhost:11434/v1".into(),
            model: "llama3".into(),
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
    fn a_save_keeps_the_learned_json_mode_until_the_address_changes() {
        let (_dir, core) = core();
        let first = save_provider(&core, &input(0, None)).unwrap();
        let mut row = core.store.get_provider(first.id).unwrap().unwrap();
        row.json_mode = "json_object".into();
        core.store.update_provider(&row).unwrap();

        save_provider(&core, &ProviderInput { name: "Renamed".into(), ..input(first.id, None) }).unwrap();
        assert_eq!(core.store.get_provider(first.id).unwrap().unwrap().json_mode, "json_object");

        let moved = ProviderInput { base_url: "http://localhost:8080/v1".into(), ..input(first.id, None) };
        save_provider(&core, &moved).unwrap();
        assert_eq!(core.store.get_provider(first.id).unwrap().unwrap().json_mode, "json_schema");
    }

    #[test]
    fn a_codex_provider_signs_in_instead_of_keeping_a_key() {
        let (_dir, core) = core();
        let codex = |id: i64, key: Option<&str>| ProviderInput {
            kind: "codex".into(),
            name: "Codex".into(),
            base_url: String::new(),
            model: String::new(),
            ..input(id, key)
        };
        // The engine says what to do (the shell has no checks of its own).
        assert_eq!(core.codex_import_cli_login(0).unwrap_err().to_string(), "Save the provider, then sign in.");

        let saved = save_provider(&core, &codex(0, Some(API_KEY))).unwrap();
        assert_eq!((saved.kind, saved.account.as_deref()), ("codex", None));
        assert_eq!(saved.base_url, jobpilot_engine::llm::default_base_url(jobpilot_engine::store::ProviderKind::Codex));
        assert_eq!(core.provider_api_key(saved.id).unwrap(), None, "a key typed for another kind is not kept");
        assert!(!provider_has_key(&core, saved.id), "not signed in");

        let other = save_provider(&core, &input(0, None)).unwrap();
        assert_eq!(
            core.codex_import_cli_login(other.id).unwrap_err().to_string(),
            "Only a Codex provider signs in with ChatGPT."
        );
    }

    #[test]
    fn a_saved_key_goes_only_to_the_endpoint_it_was_saved_for() {
        let (_dir, core) = core();
        let saved = save_provider(&core, &input(0, Some(API_KEY))).unwrap();
        let key = |form: &ProviderInput| {
            let (provider, row) = provider_of(&core, form).unwrap();
            key_for(&core, &provider, row.as_ref(), form.api_key.clone()).unwrap()
        };
        assert_eq!(key(&input(saved.id, None)).as_deref(), Some(API_KEY), "same kind and address");
        let moved = ProviderInput { base_url: "https://evil.example/v1".into(), ..input(saved.id, None) };
        assert_eq!(key(&moved), None, "another address: no saved key");
        let other_kind = ProviderInput { kind: "anthropic_compatible".into(), ..input(saved.id, None) };
        assert_eq!(key(&other_kind), None, "another kind: no saved key");
        let official = ProviderInput { kind: "openai".into(), ..input(saved.id, None) };
        assert_eq!(key(&official), None, "an official kind is another host");
        let typed = ProviderInput { base_url: "https://evil.example/v1".into(), ..input(saved.id, Some("sk-typed")) };
        assert_eq!(key(&typed).as_deref(), Some("sk-typed"));
        assert!(client_for(&core, &moved).is_ok());

        // Saving the moved form without a key forgets the saved one.
        save_provider(&core, &moved).unwrap();
        assert_eq!(core.provider_api_key(saved.id).unwrap(), None);
        assert!(!provider_has_key(&core, saved.id));
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
    fn the_browser_section_lists_the_sites_and_says_no_browser_is_connected() {
        let (_dir, core) = core();
        let site = core.sessions().create("LinkedIn", "https://www.linkedin.com/login").unwrap();
        let dto = browser_section(&core, None).unwrap();
        assert!(!dto.browser.connected && dto.browser.pairing_code.is_none());
        assert_eq!(dto.sessions.len(), 1);
        assert_eq!((dto.sessions[0].id, dto.sessions[0].status), (site.id, "new"));
        assert!(!dto.sessions[0].login_in_progress);
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
