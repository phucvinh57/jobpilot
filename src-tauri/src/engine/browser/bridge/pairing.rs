//! Pairing: the app and the extension prove to each other that they hold the token the user paired
//! them with, without sending it.
//!
//! The token is 32 random bytes (hex), kept in the OS keyring ([`TOKEN_KEY`]) and in the extension's
//! `chrome.storage.local`. It travels once, in `paired`. On every later connection each side sends a
//! random nonce and the other answers with HMAC-SHA256 keyed by the token over a prefixed nonce
//! ([`app_proof`]: `jobpilot-app:<extension nonce>`, [`ext_proof`]: `jobpilot-ext:<app nonce>`; the
//! prefixes keep one side's answer from being replayed as the other's). Proofs are checked in
//! constant time. An extension without the token gets a 6-digit code, shown both in its popup and in
//! the app, and the user approves it on both sides. Tokens, nonces and proofs are never logged.

use std::time::Duration;

use hmac::{Hmac, Mac};
use rand::{Rng, RngCore};
use sha2::Sha256;

/// The keyring entry of the token.
pub const TOKEN_KEY: &str = "browser.extension.token";

/// How long a pairing request waits for the user.
pub const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);

const APP: &str = "jobpilot-app:";
const EXT: &str = "jobpilot-ext:";

/// 32 random bytes as 64 lower-case hex characters.
fn random_hex() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex(&bytes)
}

/// A new token: 32 random bytes as 64 lower-case hex characters.
pub fn new_token() -> String {
    random_hex()
}

/// A new nonce: 32 random bytes as 64 lower-case hex characters.
pub fn new_nonce() -> String {
    random_hex()
}

/// Is `nonce` what a nonce looks like (64 hex characters)? Anything else is not answered.
pub fn is_nonce(nonce: &str) -> bool {
    nonce.len() == 64 && nonce.bytes().all(|b| b.is_ascii_hexdigit())
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| text.get(i..i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok())).collect()
}

/// HMAC-SHA256 keyed by the token's characters over `prefix` + `nonce`; `None` for an empty token.
fn mac(token: &str, prefix: &str, nonce: &str) -> Option<Hmac<Sha256>> {
    if token.is_empty() {
        return None;
    }
    // HMAC takes a key of any length, so this only fails for the empty token refused above.
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes()).ok()?;
    mac.update(prefix.as_bytes());
    mac.update(nonce.as_bytes());
    Some(mac)
}

/// The app's answer to the extension's nonce: hex(HMAC-SHA256(token, "jobpilot-app:" + nonce)).
/// Empty for an empty token (which never verifies).
pub fn app_proof(token: &str, ext_nonce: &str) -> String {
    mac(token, APP, ext_nonce).map(|m| hex(&m.finalize().into_bytes())).unwrap_or_default()
}

/// The extension's answer to the app's nonce: hex(HMAC-SHA256(token, "jobpilot-ext:" + nonce)).
pub fn ext_proof(token: &str, app_nonce: &str) -> String {
    mac(token, EXT, app_nonce).map(|m| hex(&m.finalize().into_bytes())).unwrap_or_default()
}

/// Is `proof` the extension's answer to `app_nonce` for `token`? Constant time.
pub fn verify_ext_proof(token: &str, app_nonce: &str, proof: &str) -> bool {
    let (Some(mac), Some(proof)) = (mac(token, EXT, app_nonce), unhex(proof)) else { return false };
    mac.verify_slice(&proof).is_ok()
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
    }

    /// The same vector is checked by the extension (`extension/src/auth.test.ts`).
    const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const APP_PROOF: &str = "f121701b6248be0864763d4ba74c37535456af3125ef7e9d700c0a4eb862dbd0";
    const EXT_PROOF: &str = "2cb63fe35b8ba5cc761110c06bef34353c149c6eebd169371c6cdcd15eb28f74";

    #[test]
    fn proofs_match_the_shared_test_vector() {
        let nonce = "0f".repeat(32);
        assert_eq!(app_proof(TOKEN, &nonce), APP_PROOF);
        assert_eq!(ext_proof(TOKEN, &nonce), EXT_PROOF);
        assert!(verify_ext_proof(TOKEN, &nonce, EXT_PROOF));
        assert!(!verify_ext_proof(TOKEN, &nonce, APP_PROOF), "the app's answer is not the extension's");
        assert!(!verify_ext_proof(TOKEN, &"0e".repeat(32), EXT_PROOF), "another nonce");
        assert!(!verify_ext_proof(&"1".repeat(64), &nonce, EXT_PROOF), "another token");
        assert!(!verify_ext_proof(TOKEN, &nonce, &EXT_PROOF[..62]), "a short proof");
        assert!(!verify_ext_proof(TOKEN, &nonce, "zz"), "not hex");
        assert!(!verify_ext_proof(TOKEN, &nonce, ""));
        assert!(!verify_ext_proof("", &nonce, &ext_proof("", &nonce)), "no token, no proof");
        assert_eq!(app_proof("", &nonce), "");
    }

    #[test]
    fn nonces_are_64_hex_and_differ() {
        let (a, b) = (new_nonce(), new_nonce());
        assert!(is_nonce(&a) && is_nonce(&b));
        assert_ne!(a, b);
        assert!(!is_nonce("abc") && !is_nonce(&"g".repeat(64)) && !is_nonce(""));
    }
}
