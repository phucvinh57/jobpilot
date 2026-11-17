//! Sample answers for UI work (`JOBPILOT_DEMO=1`): a job page on `example.com` points to a
//! Greenhouse board, so the add form's suggestion can be seen without the network, and is added
//! without the check that reads it; "Suggest sources" finds three canned sources without any
//! search. Nothing is fetched.

use url::Url;

use crate::engine::discover::source::{detect_boards, BoardHint};
use crate::engine::store::NewSuggestion;

/// The page every `example.com` job page "is" in demo mode.
const DEMO_PAGE: &str = r#"<iframe title="Jobs" src="https://boards.greenhouse.io/embed/job_board?for=example"></iframe>"#;

/// The board hints of `url` in demo mode: `Some` only with `JOBPILOT_DEMO` set and a page on
/// `example.com`; `None` means "ask the real page".
pub fn board_hints(url: &str) -> Option<Vec<BoardHint>> {
    std::env::var_os("JOBPILOT_DEMO")?;
    demo_hints(url)
}

fn demo_hints(url: &str) -> Option<Vec<BoardHint>> {
    let parsed = example_page(url)?;
    Some(detect_boards(DEMO_PAGE, &parsed))
}

/// In demo mode, a job page on `example.com` is added without the check that reads it (the
/// address is still checked).
pub fn skips_check(kind: &str, url: &str) -> bool {
    std::env::var_os("JOBPILOT_DEMO").is_some() && demo_skips_check(kind, url)
}

fn demo_skips_check(kind: &str, url: &str) -> bool {
    kind.trim() == "career_page" && example_page(url).is_some()
}

/// What "Suggest sources" finds in demo mode: `Some` only with `JOBPILOT_DEMO` set; `None` means
/// "search the web".
pub fn suggestions() -> Option<Vec<NewSuggestion>> {
    std::env::var_os("JOBPILOT_DEMO")?;
    Some(demo_suggestions())
}

fn demo_suggestions() -> Vec<NewSuggestion> {
    let one = |kind: &str, input: &str, name: &str, jobs: u32, matching: u32| NewSuggestion {
        kind: kind.into(),
        input: input.into(),
        name: name.into(),
        jobs,
        matching,
        query: "demo".into(),
    };
    vec![
        one("greenhouse", "boards.greenhouse.io/demo-robotics", "Demo Robotics", 14, 3),
        one("lever", "jobs.lever.co/demo-health", "Demo Health", 6, 2),
        one("career_page", "https://example.com/careers", "example.com", 4, 4),
    ]
}

/// `url` as an address on `example.com` (or a subdomain of it); `None` for any other.
fn example_page(url: &str) -> Option<Url> {
    let raw = url.trim();
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let parsed = Url::parse(&with_scheme).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    (host == "example.com" || host.ends_with(".example.com")).then_some(parsed)
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

    #[test]
    fn the_demo_suggestions_pass_the_address_check() {
        let list = demo_suggestions();
        assert_eq!(list.len(), 3);
        for s in &list {
            assert!(crate::engine::discover::source::check_source(&s.kind, &s.input).is_ok(), "{}", s.input);
        }
    }

    #[test]
    fn only_example_com_job_pages_skip_the_check() {
        assert!(demo_skips_check("career_page", "https://careers.example.com/jobs"));
        assert!(demo_skips_check(" career_page ", "example.com/jobs"));
        assert!(!demo_skips_check("greenhouse", "https://example.com/jobs"));
        for other in ["https://acme.test/careers", "https://example.com.evil.test/", "not a url"] {
            assert!(!demo_skips_check("career_page", other), "{other}");
        }
    }
}
