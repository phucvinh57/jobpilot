//! Read models of this context (serde, camelCase). Core types stay free of serde and Tauri.
//!
//! No secret is ever in here: an API key or an OAuth secret goes in through [`ProviderInput`] or a
//! command argument and straight to the keyring; what comes back is at most a `has...` flag.

use std::fmt;

use crate::engine::browser::BridgeStatus;
use crate::engine::llm;
use crate::engine::mail::{GmailState, ReportSummary};
use crate::engine::store::{Provider, ProviderKind, Session};
use serde::{Deserialize, Serialize};

use super::rules::{host, kind_defaults, kind_label, KINDS};

// ------------------------------------------------------------------ providers

/// A saved AI provider. Its key is not here: see `integrations_provider_has_key`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: i64,
    pub name: String,
    /// `openai` / `anthropic`.
    pub kind: &'static str,
    pub kind_label: &'static str,
    pub base_url: String,
    pub model: String,
    /// The endpoint lacks `json_schema` support: ask for a JSON object instead.
    pub json_object: bool,
    pub is_default: bool,
}

impl From<&Provider> for ProviderDto {
    fn from(p: &Provider) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            kind: p.kind.as_str(),
            kind_label: kind_label(p.kind),
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            json_object: p.json_mode == llm::openai::JSON_MODE_OBJECT,
            is_default: p.is_default,
        }
    }
}

/// What a kind of provider starts with in the form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindDto {
    pub kind: &'static str,
    pub label: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub model: &'static str,
}

impl From<ProviderKind> for KindDto {
    fn from(kind: ProviderKind) -> Self {
        let (name, base_url, model) = kind_defaults(kind);
        Self {
            kind: kind.as_str(),
            label: kind_label(kind),
            name,
            base_url,
            model,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersDto {
    pub providers: Vec<ProviderDto>,
    pub kinds: Vec<KindDto>,
}

impl ProvidersDto {
    pub fn new(providers: &[Provider]) -> Self {
        Self {
            providers: providers.iter().map(ProviderDto::from).collect(),
            kinds: KINDS.map(KindDto::from).to_vec(),
        }
    }
}

/// The provider form, as the web app sends it to save, test or list models. The key is typed text
/// that goes to the keyring and nowhere else: this type is never serialized and never printed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    /// `0`: a provider that is not saved yet.
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub json_object: bool,
    /// Typed in the form; empty or missing keeps the saved key.
    pub api_key: Option<String>,
}

impl fmt::Debug for ProviderInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderInput")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("api_key", &self.api_key.as_ref().map(|_| "<hidden>"))
            .finish_non_exhaustive()
    }
}

// ------------------------------------------------------------------ browser

/// A site the user is signed in to in their own browser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    pub id: i64,
    pub name: String,
    pub host: String,
    /// `new` / `ok` / `expired`.
    pub status: &'static str,
    /// RFC 3339, `None` when it was never checked.
    pub verified_at: Option<String>,
    /// A login tab is open and waits for the user to press Done.
    pub login_in_progress: bool,
}

impl SessionDto {
    pub fn new(session: &Session, login_in_progress: bool) -> Self {
        Self {
            id: session.id,
            name: session.name.clone(),
            host: host(&session.start_url),
            status: session.session_status().as_str(),
            verified_at: session.last_verified_at.map(|t| t.to_rfc3339()),
            login_in_progress,
        }
    }
}

/// The user's browser as the Browser section shows it: whether the JobPilot extension is
/// connected and paired. Never the pairing token (only the 6-digit code the user compares).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDto {
    /// The app waits for the extension (false when its port is taken).
    pub listening: bool,
    pub port: u16,
    /// A paired extension is connected.
    pub connected: bool,
    /// "Chrome 153", also while pairing.
    pub browser: Option<String>,
    pub extension_version: Option<String>,
    /// The extension may open private (Incognito) tabs.
    pub incognito_allowed: bool,
    /// The code to approve while an extension waits for pairing.
    pub pairing_code: Option<String>,
    /// The user approved the code here; the extension's popup still has to say Pair.
    pub pairing_waiting_for_extension: bool,
    /// Some browser was paired before.
    pub paired: bool,
    /// The folder to pick in "Load unpacked" on chrome://extensions, when it is known.
    pub extension_path: Option<String>,
}

impl BrowserDto {
    pub fn new(status: &BridgeStatus, extension_path: Option<String>) -> Self {
        Self {
            listening: status.listening,
            port: status.port,
            connected: status.connected,
            browser: status.browser.clone(),
            extension_version: status.extension_version.clone(),
            incognito_allowed: status.incognito_allowed,
            pairing_code: status.pairing_code.clone(),
            pairing_waiting_for_extension: status.pairing_waiting_for_extension,
            paired: status.paired,
            extension_path,
        }
    }
}

/// The Browser section: the connection and the sites the user is signed in to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSectionDto {
    pub browser: BrowserDto,
    pub sessions: Vec<SessionDto>,
}

// ------------------------------------------------------------------ gmail

/// The last sync, as the settings card shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReportDto {
    pub fetched: usize,
    pub updated: usize,
    pub needs_review: usize,
    pub error: Option<String>,
}

impl From<&ReportSummary> for SyncReportDto {
    fn from(r: &ReportSummary) -> Self {
        Self {
            fetched: r.fetched,
            updated: r.updated,
            needs_review: r.needs_review,
            error: r.error.clone(),
        }
    }
}

/// Gmail settings and sync status. The client secret and the refresh token stay in the keyring:
/// only whether a secret is saved is here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailDto {
    pub client_id: String,
    pub has_client_secret: bool,
    /// The connected account's address.
    pub account: Option<String>,
    /// RFC 3339.
    pub last_sync: Option<String>,
    pub last_report: Option<SyncReportDto>,
    /// Minutes between polls while the app is open.
    pub poll_minutes: u64,
}

impl GmailDto {
    pub fn new(state: &GmailState, has_client_secret: bool, poll_minutes: u64) -> Self {
        Self {
            client_id: state.client_id.clone(),
            has_client_secret,
            account: state.account.clone(),
            last_sync: state.last_sync.map(|t| t.to_rfc3339()),
            last_report: state.last_report.as_ref().map(SyncReportDto::from),
            poll_minutes,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use crate::engine::mail::{GmailState, ReportSummary};
    use crate::engine::store::{Provider, ProviderKind, Session};
    use serde_json::Value;

    use crate::engine::browser::BridgeStatus;

    use super::{BrowserDto, GmailDto, ProviderDto, ProviderInput, ProvidersDto, SessionDto};

    fn provider() -> Provider {
        Provider {
            id: 3,
            name: "Local".into(),
            kind: ProviderKind::OpenAi,
            base_url: "http://localhost:11434/v1".into(),
            model: "llama3".into(),
            json_mode: crate::engine::llm::openai::JSON_MODE_OBJECT.into(),
            is_default: true,
        }
    }

    /// Every key of `value` that names a key, secret or token holds a flag, never text.
    fn assert_no_secret_fields(value: &Value, path: &str) {
        match value {
            Value::Object(map) => {
                for (name, inner) in map {
                    let lower = name.to_lowercase();
                    if ["key", "secret", "token", "password"]
                        .iter()
                        .any(|w| lower.contains(w))
                    {
                        assert!(
                            inner.is_boolean() && lower.starts_with("has"),
                            "{path}.{name} may only be a has... flag, found {inner}"
                        );
                    }
                    assert_no_secret_fields(inner, &format!("{path}.{name}"));
                }
            }
            Value::Array(items) => items
                .iter()
                .enumerate()
                .for_each(|(i, v)| assert_no_secret_fields(v, &format!("{path}[{i}]"))),
            _ => {}
        }
    }

    #[test]
    fn a_provider_carries_no_key_field() {
        let dto = ProviderDto::from(&provider());
        let json = serde_json::to_value(&dto).unwrap();
        assert_no_secret_fields(&json, "provider");
        assert_eq!(json["kind"], "openai");
        assert_eq!(json["kindLabel"], "OpenAI-compatible");
        assert_eq!(json["jsonObject"], true);
        assert_eq!(json["isDefault"], true);
        assert!(json.get("apiKey").is_none());

        let all = serde_json::to_value(ProvidersDto::new(&[provider()])).unwrap();
        assert_no_secret_fields(&all, "providers");
        assert_eq!(all["kinds"][1]["kind"], "anthropic");
        assert_eq!(all["kinds"][1]["name"], "Anthropic");
    }

    #[test]
    fn gmail_carries_no_secret_field() {
        let state = GmailState {
            client_id: "abc.apps.googleusercontent.com".into(),
            account: Some("me@example.com".into()),
            last_sync: Some(Utc.with_ymd_and_hms(2026, 11, 14, 9, 5, 0).unwrap()),
            last_report: Some(ReportSummary {
                fetched: 4,
                updated: 1,
                needs_review: 2,
                error: None,
            }),
        };
        let dto = GmailDto::new(&state, true, 10);
        let json = serde_json::to_value(&dto).unwrap();
        assert_no_secret_fields(&json, "gmail");
        assert_eq!(json["hasClientSecret"], true);
        assert_eq!(json["account"], "me@example.com");
        assert_eq!(json["lastReport"]["needsReview"], 2);
        assert_eq!(json["pollMinutes"], 10);
        assert!(json["lastSync"]
            .as_str()
            .is_some_and(|s| s.starts_with("2026-11-14T09:05:00")));
        assert!(json.get("clientSecret").is_none() && json.get("refreshToken").is_none());
    }

    #[test]
    fn a_session_shows_its_host_and_state() {
        let session = Session {
            id: 2,
            name: "LinkedIn".into(),
            start_url: "https://www.linkedin.com/login".into(),
            domains_json: "[]".into(),
            status: "ok".into(),
            last_verified_at: None,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        };
        let json = serde_json::to_value(SessionDto::new(&session, true)).unwrap();
        assert_eq!(
            (json["host"].as_str(), json["status"].as_str()),
            (Some("www.linkedin.com"), Some("ok"))
        );
        assert_eq!(json["loginInProgress"], true);
        assert!(json["verifiedAt"].is_null());
    }

    #[test]
    fn the_browser_carries_the_pairing_code_and_no_token() {
        let status = BridgeStatus {
            listening: true,
            port: 47615,
            connected: false,
            browser: Some("Chrome 153".into()),
            extension_version: Some("0.1.0".into()),
            incognito_allowed: false,
            pairing_code: Some("123456".into()),
            pairing_waiting_for_extension: true,
            paired: true,
        };
        let json = serde_json::to_value(BrowserDto::new(&status, Some("/opt/jobpilot/extension".into()))).unwrap();
        assert_no_secret_fields(&json, "browser");
        assert_eq!(json["pairingCode"], "123456");
        assert_eq!(json["pairingWaitingForExtension"], true);
        assert_eq!(json["extensionVersion"], "0.1.0");
        assert_eq!(json["extensionPath"], "/opt/jobpilot/extension");
        assert_eq!((json["connected"].as_bool(), json["incognitoAllowed"].as_bool()), (Some(false), Some(false)));
    }

    #[test]
    fn the_typed_key_is_never_printed() {
        let input: ProviderInput = serde_json::from_str(
            r#"{"id":0,"kind":"openai","name":"X","baseUrl":"","model":"","jsonObject":false,"apiKey":"sk-VERY-SECRET-1234"}"#,
        )
        .unwrap();
        assert_eq!(input.api_key.as_deref(), Some("sk-VERY-SECRET-1234"));
        let printed = format!("{input:?}");
        assert!(!printed.contains("VERY-SECRET"), "{printed}");
        assert!(printed.contains("<hidden>"));
        let without: ProviderInput = serde_json::from_str(
            r#"{"id":1,"kind":"openai","name":"X","baseUrl":"","model":"","jsonObject":false}"#,
        )
        .unwrap();
        assert!(
            without.api_key.is_none(),
            "no key typed: the saved one is kept"
        );
    }
}
