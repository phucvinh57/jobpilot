//! Sample answers for UI work (`JOBPILOT_DEMO=1`): a careers page on `example.com` points to a
//! Greenhouse board, so the add form's suggestion can be seen without the network. Nothing is
//! fetched.

use url::Url;

use crate::engine::discover::source::{detect_boards, BoardHint};

/// The page every `example.com` careers page "is" in demo mode.
const DEMO_PAGE: &str = r#"<iframe title="Jobs" src="https://boards.greenhouse.io/embed/job_board?for=example"></iframe>"#;

/// The board hints of `url` in demo mode: `Some` only with `JOBPILOT_DEMO` set and a page on
/// `example.com`; `None` means "ask the real page".
pub fn board_hints(url: &str) -> Option<Vec<BoardHint>> {
    std::env::var_os("JOBPILOT_DEMO")?;
    demo_hints(url)
}

fn demo_hints(url: &str) -> Option<Vec<BoardHint>> {
    let raw = url.trim();
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let parsed = Url::parse(&with_scheme).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    (host == "example.com" || host.ends_with(".example.com")).then(|| detect_boards(DEMO_PAGE, &parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_example_com_pages_get_the_sample_hint() {
        let hints = demo_hints("careers.example.com/jobs").unwrap();
        assert_eq!(hints, [BoardHint { kind: "greenhouse", input: "boards.greenhouse.io/example".into() }]);
        for other in ["https://acme.test/careers", "https://example.com.evil.test/", "not a url"] {
            assert_eq!(demo_hints(other), None, "{other}");
        }
    }
}
