//! Pure rules of this context, unit-tested here: what a provider form means as a provider, what
//! each kind starts with, which host a session shows, where the extension to load is.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, ensure, Result};
use jobpilot_engine::llm;
use jobpilot_engine::store::{Provider, ProviderKind};

/// What the type picker shows.
pub fn kind_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAiCompatible => "OpenAI-compatible",
        ProviderKind::AnthropicCompatible => "Anthropic-compatible",
        ProviderKind::OpenAi => "OpenAI",
        ProviderKind::Anthropic => "Anthropic",
        ProviderKind::Codex => "Codex (ChatGPT)",
    }
}

/// Name, base URL and model prefilled for a kind (the compatible kinds have no base URL or model:
/// the user enters them).
pub fn kind_defaults(kind: ProviderKind) -> (&'static str, &'static str, &'static str) {
    let name = match kind {
        ProviderKind::OpenAiCompatible => "Local model",
        ProviderKind::AnthropicCompatible => "Anthropic-compatible",
        ProviderKind::OpenAi => "OpenAI",
        ProviderKind::Anthropic => "Anthropic",
        ProviderKind::Codex => "Codex",
    };
    (name, llm::default_base_url(kind), llm::default_model(kind))
}

/// The provider kinds in the order the picker shows them.
pub const KINDS: [ProviderKind; 5] = ProviderKind::ALL;

/// The kind needs the user's base URL (the compatible kinds); the others have a fixed address.
pub fn needs_base_url(kind: ProviderKind) -> bool {
    !kind.is_official()
}

/// The kind is reached with an API key (every kind but Codex, which signs in with ChatGPT).
pub fn uses_key(kind: ProviderKind) -> bool {
    kind != ProviderKind::Codex
}

/// The kind signs in instead of taking a key (Codex).
pub fn signs_in(kind: ProviderKind) -> bool {
    kind == ProviderKind::Codex
}

/// The provider a form describes (`id == 0`: not saved yet; `saved`: the stored row of `id`). The
/// name is required; a compatible kind needs a base URL, an official kind always uses its own. The
/// JSON mode is never the user's: it is the one the engine learned for the saved row, and starts
/// over at `json_schema` for a new provider or once the kind or the address changed.
pub fn provider_from_form(
    id: i64,
    kind: &str,
    name: &str,
    base_url: &str,
    model: &str,
    saved: Option<&Provider>,
) -> Result<Provider> {
    let kind = ProviderKind::parse(kind).ok_or_else(|| anyhow!("unknown provider type: {kind}"))?;
    let name = name.trim();
    ensure!(!name.is_empty(), "Enter a name.");
    let base_url = if needs_base_url(kind) {
        let typed = base_url.trim();
        ensure!(!typed.is_empty(), "Enter the base URL.");
        typed.to_string()
    } else {
        llm::default_base_url(kind).to_string()
    };
    let json_mode = match saved {
        Some(row) if row.kind == kind && row.base_url.trim() == base_url && !row.json_mode.is_empty() => {
            row.json_mode.clone()
        }
        _ => llm::openai::JSON_MODE_SCHEMA.to_string(),
    };
    Ok(Provider {
        id,
        name: name.to_string(),
        kind,
        base_url,
        model: model.trim().to_string(),
        json_mode,
        is_default: false,
    })
}

/// The key typed into a form: `None` when the field was left empty (keep the saved one).
pub fn typed_key(key: Option<String>) -> Option<String> {
    key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

/// The OAuth client id of the Gmail form; it is required.
pub fn client_id(typed: &str) -> Result<String> {
    let id = typed.trim();
    ensure!(!id.is_empty(), "Enter the OAuth client ID.");
    Ok(id.to_string())
}

/// Host shown under the session name.
pub fn host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// The folder of the built extension to pick in "Load unpacked". Windows uses the copy next to
/// the executable first so installed apps can be moved between machines and upgraded in place.
/// Other platforms use the source checkout first. Only a folder with a `manifest.json` counts.
pub fn extension_path(checkout: &Path, bundled: Option<PathBuf>) -> Option<PathBuf> {
    let checkout = Some(checkout.to_path_buf());
    let candidates = if cfg!(windows) {
        [bundled, checkout]
    } else {
        [checkout, bundled]
    };
    candidates
        .into_iter()
        .flatten()
        .find(|dir| dir.join("manifest.json").is_file())
}

#[cfg(test)]
mod tests {
    use jobpilot_engine::store::{Provider, ProviderKind};

    use super::{
        client_id, host, kind_defaults, kind_label, needs_base_url, provider_from_form, signs_in, typed_key, uses_key,
        KINDS,
    };

    fn saved(kind: ProviderKind, base_url: &str, json_mode: &str) -> Provider {
        Provider {
            id: 7,
            name: "Saved".into(),
            kind,
            base_url: base_url.into(),
            model: "m".into(),
            json_mode: json_mode.into(),
            is_default: true,
        }
    }

    #[test]
    fn a_compatible_form_becomes_a_provider_with_its_base_url() {
        for kind in ["openai_compatible", "anthropic_compatible"] {
            let p = provider_from_form(0, kind, "  Local  ", " http://localhost:11434/v1 ", " llama3 ", None).unwrap();
            assert_eq!(
                (p.id, p.name.as_str(), p.base_url.as_str(), p.model.as_str()),
                (0, "Local", "http://localhost:11434/v1", "llama3")
            );
            assert_eq!(p.kind.as_str(), kind);
            assert_eq!(p.json_mode, "json_schema", "a new provider starts with json_schema");
            assert!(!p.is_default, "the default flag is set by its own action");
            assert_eq!(
                provider_from_form(0, kind, "Local", "   ", "llama3", None).unwrap_err().to_string(),
                "Enter the base URL."
            );
        }
    }

    #[test]
    fn an_official_kind_always_uses_its_own_address() {
        for kind in [ProviderKind::OpenAi, ProviderKind::Anthropic, ProviderKind::Codex] {
            let typed = provider_from_form(0, kind.as_str(), "X", "http://elsewhere.example/v1", "", None).unwrap();
            let empty = provider_from_form(0, kind.as_str(), "X", "", "", None).unwrap();
            let own = jobpilot_engine::llm::default_base_url(kind);
            assert!(own.starts_with("https://"), "{kind:?}: {own}");
            assert_eq!((typed.base_url.as_str(), empty.base_url.as_str()), (own, own), "{kind:?}");
            assert_eq!(typed.kind, kind);
        }
    }

    #[test]
    fn the_learned_json_mode_is_kept_until_the_kind_or_the_address_changes() {
        let row = saved(ProviderKind::OpenAiCompatible, "http://localhost:11434/v1", "json_object");
        let same = provider_from_form(7, "openai_compatible", "Local", " http://localhost:11434/v1 ", "llama3", Some(&row)).unwrap();
        assert_eq!(same.json_mode, "json_object", "an edit of the name or model keeps what was learned");

        let moved = provider_from_form(7, "openai_compatible", "Local", "http://localhost:8080/v1", "llama3", Some(&row)).unwrap();
        assert_eq!(moved.json_mode, "json_schema", "another address is asked again");

        let other_kind = provider_from_form(7, "anthropic_compatible", "Local", "http://localhost:11434/v1", "", Some(&row)).unwrap();
        assert_eq!(other_kind.json_mode, "json_schema", "another kind is asked again");

        let official = saved(ProviderKind::OpenAi, "https://api.openai.com/v1", "prompt");
        let kept = provider_from_form(7, "openai", "OpenAI", "", "gpt-4o", Some(&official)).unwrap();
        assert_eq!(kept.json_mode, "prompt");
    }

    #[test]
    fn a_form_without_a_name_or_with_an_unknown_type_is_refused() {
        assert_eq!(
            provider_from_form(0, "openai", "   ", "", "", None)
                .unwrap_err()
                .to_string(),
            "Enter a name."
        );
        assert!(provider_from_form(0, "gemini", "G", "", "", None).is_err());
    }

    #[test]
    fn a_key_left_empty_keeps_the_saved_one() {
        assert_eq!(typed_key(None), None);
        assert_eq!(typed_key(Some("   ".into())), None);
        assert_eq!(typed_key(Some(" sk-1 ".into())), Some("sk-1".to_string()));
    }

    #[test]
    fn every_kind_has_a_label_and_defaults() {
        assert_eq!(
            KINDS.map(kind_label),
            ["OpenAI-compatible", "Anthropic-compatible", "OpenAI", "Anthropic", "Codex (ChatGPT)"]
        );
        for kind in KINDS {
            let (name, url, _model) = kind_defaults(kind);
            assert!(!name.is_empty(), "{kind:?}");
            assert_eq!(url.is_empty(), needs_base_url(kind), "{kind:?}: {url}");
            assert!(url.is_empty() || url.starts_with("https://"), "{kind:?}: {url}");
        }
        assert_eq!(kind_defaults(ProviderKind::OpenAiCompatible).0, "Local model");
        assert_eq!(kind_defaults(ProviderKind::Codex).0, "Codex");
    }

    #[test]
    fn only_codex_signs_in_instead_of_taking_a_key() {
        for kind in KINDS {
            assert_eq!(uses_key(kind), kind != ProviderKind::Codex, "{kind:?}");
            assert_eq!(signs_in(kind), kind == ProviderKind::Codex, "{kind:?}");
        }
        assert!(needs_base_url(ProviderKind::OpenAiCompatible) && needs_base_url(ProviderKind::AnthropicCompatible));
        assert!(!needs_base_url(ProviderKind::OpenAi) && !needs_base_url(ProviderKind::Codex));
    }

    #[test]
    fn the_gmail_client_id_is_required() {
        assert_eq!(
            client_id(" abc.apps.googleusercontent.com ").unwrap(),
            "abc.apps.googleusercontent.com"
        );
        assert_eq!(
            client_id("  ").unwrap_err().to_string(),
            "Enter the OAuth client ID."
        );
    }

    #[test]
    fn a_session_shows_the_host_of_its_login_address() {
        assert_eq!(host("https://www.linkedin.com/login"), "www.linkedin.com");
        assert_eq!(host("https://accounts.google.com/"), "accounts.google.com");
        assert_eq!(host("linkedin.com/login"), "linkedin.com", "no scheme");
        assert_eq!(host("http://localhost:8080"), "localhost:8080");
    }

    #[test]
    fn the_extension_folder_has_a_manifest_and_follows_platform_preference() {
        use super::extension_path;
        let dir = tempfile::tempdir().unwrap();
        let (checkout, bundled) = (dir.path().join("repo/extension/dist"), dir.path().join("resources/extension"));
        assert_eq!(extension_path(&checkout, Some(bundled.clone())), None, "nothing built");
        std::fs::create_dir_all(&bundled).unwrap();
        assert_eq!(extension_path(&checkout, Some(bundled.clone())), None, "an empty folder is not the extension");
        std::fs::write(bundled.join("manifest.json"), "{}").unwrap();
        assert_eq!(extension_path(&checkout, Some(bundled.clone())), Some(bundled.clone()));
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(checkout.join("manifest.json"), "{}").unwrap();
        let preferred = if cfg!(windows) {
            bundled.clone()
        } else {
            checkout.clone()
        };
        assert_eq!(extension_path(&checkout, Some(bundled)), Some(preferred));
        assert_eq!(extension_path(&checkout, None), Some(checkout));
    }
}
