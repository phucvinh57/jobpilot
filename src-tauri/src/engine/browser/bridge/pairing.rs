//! Pairing: the extension proves it is the browser the user approved with a token the app made.
//!
//! The token is 32 random bytes (hex), kept in the OS keyring ([`TOKEN_KEY`]) and in the extension's
//! `chrome.storage.local`; it is compared in constant time. An extension without the token gets a
//! 6-digit code, shown both in its popup and in the app, and the user approves it in the app.

use std::time::Duration;

use rand::{Rng, RngCore};

/// The keyring entry of the token.
pub const TOKEN_KEY: &str = "browser.extension.token";

/// How long a pairing request waits for the user.
pub const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);

/// A new token: 32 random bytes as 64 lower-case hex characters.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A new 6-digit pairing code.
pub fn new_code() -> String {
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32))
}

/// `a == b` in time that depends only on the lengths.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Does the offered token match the stored one?
pub fn matches(stored: Option<&str>, offered: Option<&str>) -> bool {
    match (stored, offered) {
        (Some(stored), Some(offered)) if !stored.is_empty() => ct_eq(stored.as_bytes(), offered.as_bytes()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_hex_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..100 {
            let code = new_code();
            assert_eq!(code.len(), 6, "{code}");
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(ct_eq(b"", b""));
        assert!(matches(Some("t"), Some("t")));
        assert!(!matches(Some("t"), Some("u")));
        assert!(!matches(None, Some("t")));
        assert!(!matches(Some("t"), None));
        assert!(!matches(Some(""), Some("")), "an empty stored token never matches");
    }
}
