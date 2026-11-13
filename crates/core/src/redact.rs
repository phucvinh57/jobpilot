//! Keeping secrets out of logs and error text.
//!
//! Two layers: patterns for well-known credential shapes (`Bearer ...`, `sk-...`, `ya29....`,
//! `refresh_token=...`, JSON `"api_key": "..."`), and exact values registered with [`register`]
//! (every secret read from or written to [`crate::Secrets`] is registered, so a key that ends up in
//! an unexpected error message is still scrubbed). The log writers in `logging` run every line
//! through [`redact`].

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

use regex::Regex;

/// What replaces a secret.
pub const MASK: &str = "[REDACTED]";
/// Shorter values are not registered: they would mangle ordinary text.
const MIN_SECRET_LEN: usize = 8;

static SECRETS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Remember an exact secret value so [`redact`] removes it wherever it shows up.
pub fn register(value: &str) {
    let value = value.trim();
    if value.len() >= MIN_SECRET_LEN {
        SECRETS.lock().unwrap_or_else(|e| e.into_inner()).insert(value.to_string());
    }
}

struct Patterns {
    /// `name: value`, `name=value`, `"name": "value"`, `Authorization: Bearer value`.
    keyed: Regex,
    /// Credentials carried in a URL query (`?code=...`, `&key=...`).
    query: Regex,
    /// Token shapes recognisable without a name.
    shapes: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        keyed: Regex::new(
            r#"(?i)\b((?:proxy-)?authorization|x-api-key|x-goog-api-key|api[-_]?key|apikey|access[-_]?token|refresh[-_]?token|id[-_]?token|client[-_]?secret|code[-_]?verifier|password|passwd|secret|token)(["']?\s*[:=]\s*["']?)((?:bearer|basic)\s+)?[^\s"'&,;}\])]{6,}|\b(bearer|basic)(\s+)[A-Za-z0-9._~+/=-]{8,}"#,
        )
        .expect("static regex"),
        query: Regex::new(r#"(?i)([?&](?:code|key|token|api_key|access_token|refresh_token|id_token|client_secret|code_verifier)=)[^&\s"'#]+"#)
            .expect("static regex"),
        shapes: Regex::new(
            r"\b(?:sk-[A-Za-z0-9_-]{16,}|AIza[0-9A-Za-z_-]{30,}|ya29\.[0-9A-Za-z_-]{20,}|1//[0-9A-Za-z_-]{20,}|GOCSPX-[0-9A-Za-z_-]{10,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,})",
        )
        .expect("static regex"),
    })
}

/// `text` with credentials replaced by [`MASK`]. Borrows when nothing matched.
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    for secret in SECRETS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        if out.contains(secret.as_str()) {
            out = Cow::Owned(out.replace(secret.as_str(), MASK));
        }
    }
    let p = patterns();
    // `Authorization: Bearer abc` keeps the scheme visible and masks the credential only.
    let replaced = p.keyed.replace_all(&out, |c: &regex::Captures| match c.get(1) {
        Some(name) => format!("{}{}{}{}", name.as_str(), &c[2], c.get(3).map_or("", |m| m.as_str()), MASK),
        None => format!("{}{}{}", &c[4], &c[5], MASK),
    });
    if let Cow::Owned(s) = replaced {
        out = Cow::Owned(s);
    }
    let replaced = p.query.replace_all(&out, |c: &regex::Captures| format!("{}{}", &c[1], MASK));
    if let Cow::Owned(s) = replaced {
        out = Cow::Owned(s);
    }
    let replaced = p.shapes.replace_all(&out, MASK);
    if let Cow::Owned(s) = replaced {
        out = Cow::Owned(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_shapes_are_masked() {
        for secret in [
            "sk-abcdefghijklmnopqrstuvwxyz012345",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
            "ya29.a0AfH6SMBabcdefghijklmnopqrstuv",
            "1//0gabcdefghijklmnopqrstuvwxyz-ABCD",
            "GOCSPX-abcdefghijKLMNOP",
            "AIzaSyA1234567890abcdefghijklmnopqrstuv",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop",
        ] {
            let line = format!("request failed for {secret} twice");
            let out = redact(&line);
            assert!(!out.contains(secret), "{out}");
            assert!(out.contains(MASK), "{out}");
        }
    }

    #[test]
    fn named_credentials_are_masked_in_every_notation() {
        let cases = [
            ("Authorization: Bearer abcdef123456", "Authorization: Bearer [REDACTED]"),
            ("x-api-key: abcdef123456", "x-api-key: [REDACTED]"),
            (r#"{"refresh_token":"abcdef123456","ok":true}"#, r#"{"refresh_token":"[REDACTED]","ok":true}"#),
            ("GET /cb?code=4%2F0AbCd&state=x", "GET /cb?code=[REDACTED]&state=x"),
            ("refresh_token=abcdef123456&b=1", "refresh_token=[REDACTED]&b=1"),
            ("client_secret = abcdef123456", "client_secret = [REDACTED]"),
            ("api_key='abcdef123456'", "api_key='[REDACTED]'"),
            ("password: hunter2hunter2", "password: [REDACTED]"),
            ("sent Bearer abcdef123456789 upstream", "sent Bearer [REDACTED] upstream"),
            ("the OAuth token expired yesterday", "the OAuth token expired yesterday"),
        ];
        for (input, expected) in cases {
            assert_eq!(redact(input), expected, "{input}");
        }
    }

    #[test]
    fn ordinary_text_is_untouched() {
        let line = "Tailoring the CV: 3 token(s) used, model gpt-4o, applied to Acme (id 7)";
        assert!(matches!(redact(line), Cow::Borrowed(_)));
    }

    #[test]
    fn registered_secrets_are_masked_wherever_they_appear() {
        register("correct-horse-battery-staple");
        let out = redact("the server said: nope (correct-horse-battery-staple) and again correct-horse-battery-staple");
        assert!(!out.contains("correct-horse"), "{out}");
        // Too-short values are not registered (they would mangle normal text).
        register("abc");
        assert_eq!(redact("abc def"), "abc def");
    }
}
