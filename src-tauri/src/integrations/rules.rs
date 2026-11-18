//! Pure rules of this context, unit-tested here: what a provider form means as a provider, what
//! each kind starts with, which host a session shows, where the extension to load is.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, ensure, Result};
use jobpilot_engine::llm;
use jobpilot_engine::store::{Provider, ProviderKind};

/// `json_schema` is what every provider is asked for unless the endpoint cannot do it.
const JSON_MODE_SCHEMA: &str = "json_schema";

/// What the type picker shows.
pub fn kind_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "OpenAI-compatible",
        ProviderKind::Anthropic => "Anthropic",
    }
}

/// Name, base URL and model prefilled for a kind.
pub fn kind_defaults(kind: ProviderKind) -> (&'static str, &'static str, &'static str) {
    let name = match kind {
        ProviderKind::OpenAi => "OpenAI",
        ProviderKind::Anthropic => "Anthropic",
    };
    (name, llm::default_base_url(kind), llm::default_model(kind))
}

/// The provider kinds in the order the picker shows them.
pub const KINDS: [ProviderKind; 2] = [ProviderKind::OpenAi, ProviderKind::Anthropic];

/// The provider a form describes (`id == 0`: not saved yet). The name is required; an empty base
/// URL means the kind's own; `json_object` only applies to OpenAI-compatible endpoints.
pub fn provider_from_form(
    id: i64,
    kind: &str,
    name: &str,
    base_url: &str,
    model: &str,
    json_object: bool,
) -> Result<Provider> {
    let kind = ProviderKind::parse(kind).ok_or_else(|| anyhow!("unknown provider type: {kind}"))?;
    let name = name.trim();
    ensure!(!name.is_empty(), "Enter a name.");
    let base_url = match base_url.trim() {
        "" => llm::default_base_url(kind).to_string(),
        url => url.to_string(),
    };
    let json_mode = if json_object && kind == ProviderKind::OpenAi {
        llm::openai::JSON_MODE_OBJECT
    } else {
        JSON_MODE_SCHEMA
    };
    Ok(Provider {
        id,
        name: name.to_string(),
        kind,
        base_url,
        model: model.trim().to_string(),
        json_mode: json_mode.to_string(),
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
    use jobpilot_engine::store::ProviderKind;

    use super::{client_id, host, kind_defaults, kind_label, provider_from_form, typed_key, KINDS};

    #[test]
    fn a_form_becomes_a_provider() {
        let p = provider_from_form(
            0,
            "openai",
            "  Local  ",
            " http://localhost:11434/v1 ",
            " llama3 ",
            true,
        )
        .unwrap();
        assert_eq!(
            (p.id, p.name.as_str(), p.base_url.as_str(), p.model.as_str()),
            (0, "Local", "http://localhost:11434/v1", "llama3")
        );
        assert_eq!(p.kind, ProviderKind::OpenAi);
        assert_eq!(
            p.json_mode, "json_object",
            "an endpoint without json_schema support"
        );
        assert!(!p.is_default, "the default flag is set by its own action");

        let schema = provider_from_form(7, "openai", "OpenAI", "", "gpt-4o-mini", false).unwrap();
        assert_eq!(schema.json_mode, "json_schema");
        assert_eq!(
            schema.base_url,
            jobpilot_engine::llm::default_base_url(ProviderKind::OpenAi),
            "an empty base URL is the kind's own"
        );

        // json_object is an OpenAI-compatible setting only.
        let anthropic = provider_from_form(0, "anthropic", "Anthropic", "", "", true).unwrap();
        assert_eq!(anthropic.json_mode, "json_schema");
        assert_eq!(
            anthropic.base_url,
            jobpilot_engine::llm::default_base_url(ProviderKind::Anthropic)
        );
    }

    #[test]
    fn a_form_without_a_name_or_with_an_unknown_type_is_refused() {
        assert_eq!(
            provider_from_form(0, "openai", "   ", "", "", false)
                .unwrap_err()
                .to_string(),
            "Enter a name."
        );
        assert!(provider_from_form(0, "gemini", "G", "", "", false).is_err());
    }

    #[test]
    fn a_key_left_empty_keeps_the_saved_one() {
        assert_eq!(typed_key(None), None);
        assert_eq!(typed_key(Some("   ".into())), None);
        assert_eq!(typed_key(Some(" sk-1 ".into())), Some("sk-1".to_string()));
    }

    #[test]
    fn every_kind_has_a_label_and_defaults() {
        assert_eq!(KINDS.map(kind_label), ["OpenAI-compatible", "Anthropic"]);
        for kind in KINDS {
            let (name, url, _model) = kind_defaults(kind);
            assert!(
                !name.is_empty() && url.starts_with("https://"),
                "{name}: {url}"
            );
        }
        assert_eq!(kind_defaults(ProviderKind::Anthropic).0, "Anthropic");
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
