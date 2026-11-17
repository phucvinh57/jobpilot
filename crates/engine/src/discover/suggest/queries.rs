//! The web searches of "Suggest sources", built in code from the user's wanted roles and places.
//!
//! ```text
//! "Rust Engineer" Berlin site:boards.greenhouse.io
//! "Rust Engineer" Berlin site:jobs.lever.co
//! "Rust Engineer" Berlin site:jobs.ashbyhq.com
//! "Rust Engineer" jobs Berlin careers
//! ```
//!
//! The roles are the preferences' target roles; without any, the roles the profile shows
//! ([`MarketFrame`]: its visible positions and headline). The places are the preferred locations,
//! and `remote` when the user wants remote work.

use anyhow::{bail, Result};

use crate::pipeline::cv::Cv;
use crate::prefs::{Preferences, Remote};
use crate::profile::NO_PROFILE;
use crate::trends::frame::MarketFrame;

/// Most searches of one run.
pub const MAX_QUERIES: usize = 8;
/// Most places searched for one role.
const MAX_PLACES: usize = 2;
/// Longest role or place phrase put into a query.
const MAX_PHRASE_CHARS: usize = 60;
/// The job boards searched by name (each `site:` query finds that board's companies).
const BOARD_SITES: [&str; 3] = ["boards.greenhouse.io", "jobs.lever.co", "jobs.ashbyhq.com"];

/// The error when there is a profile but neither it nor the preferences name a role.
pub const NO_ROLES: &str = "no roles to search for: add the roles you want in Profile > Preferences";

/// Up to [`MAX_QUERIES`] searches: for each role and place, one per board in [`BOARD_SITES`] and one
/// for company careers pages. `cv` is the stored profile, if there is one. Without roles in the
/// preferences and without a profile, the [`NO_PROFILE`] error (`health::classify` maps it to
/// `Problem::NoProfile`); with a profile that shows no role either, [`NO_ROLES`].
pub fn build_queries(prefs: &Preferences, cv: Option<&Cv>) -> Result<Vec<String>> {
    let mut roles: Vec<String> = prefs.roles.iter().map(|r| phrase(r)).filter(|r| !r.is_empty()).collect();
    if roles.is_empty() {
        let Some(cv) = cv else { bail!(NO_PROFILE) };
        roles = MarketFrame::new(Some(cv), prefs).roles.iter().map(|r| phrase(r)).filter(|r| !r.is_empty()).collect();
    }
    if roles.is_empty() {
        bail!(NO_ROLES);
    }
    let mut places: Vec<String> = Vec::new();
    for place in prefs.locations.iter().map(|l| phrase(l)).filter(|l| !l.is_empty()) {
        if places.len() < MAX_PLACES && !places.iter().any(|p| p.eq_ignore_ascii_case(&place)) {
            places.push(place);
        }
    }
    if places.len() < MAX_PLACES && prefs.remote == Remote::Remote && !places.iter().any(|p| p.eq_ignore_ascii_case("remote")) {
        places.push("remote".into());
    }
    if places.is_empty() {
        places.push(String::new());
    }

    let mut out: Vec<String> = Vec::new();
    'all: for role in &roles {
        for place in &places {
            let quoted = format!("\"{role}\"");
            let mut forms: Vec<String> = BOARD_SITES.iter().map(|site| join(&[&quoted, place, &format!("site:{site}")])).collect();
            forms.push(join(&[&quoted, "jobs", place, "careers"]));
            for query in forms {
                if !out.contains(&query) {
                    out.push(query);
                }
                if out.len() == MAX_QUERIES {
                    break 'all;
                }
            }
        }
    }
    Ok(out)
}

/// A role or place as it goes into a query: one line, no quotes, at most [`MAX_PHRASE_CHARS`].
fn phrase(text: &str) -> String {
    let cleaned: String = text.chars().map(|c| if c == '"' || c.is_control() { ' ' } else { c }).collect();
    let one_line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(MAX_PHRASE_CHARS).collect::<String>().trim().to_string()
}

fn join(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::{classify, Problem};
    use crate::pipeline::cv::Experience;

    fn prefs(roles: &[&str], locations: &[&str], remote: Remote) -> Preferences {
        Preferences {
            roles: roles.iter().map(|s| s.to_string()).collect(),
            locations: locations.iter().map(|s| s.to_string()).collect(),
            remote,
            ..Preferences::default()
        }
    }

    #[test]
    fn one_role_and_one_place_give_three_boards_and_careers() {
        let q = build_queries(&prefs(&["Rust Engineer"], &["Berlin"], Remote::Any), None).unwrap();
        assert_eq!(
            q,
            [
                "\"Rust Engineer\" Berlin site:boards.greenhouse.io",
                "\"Rust Engineer\" Berlin site:jobs.lever.co",
                "\"Rust Engineer\" Berlin site:jobs.ashbyhq.com",
                "\"Rust Engineer\" jobs Berlin careers",
            ]
        );
    }

    #[test]
    fn roles_and_places_are_capped_at_eight_queries() {
        let q = build_queries(&prefs(&["Backend Engineer", "Data Engineer", "SRE"], &["Hanoi", "Singapore", "Tokyo"], Remote::Remote), None).unwrap();
        assert_eq!(q.len(), MAX_QUERIES);
        assert!(q[..4].iter().all(|s| s.contains("\"Backend Engineer\" ") && s.contains("Hanoi")), "{q:?}");
        assert!(q[4..].iter().all(|s| s.contains("Singapore")), "the second place before the next role: {q:?}");
        assert!(!q.iter().any(|s| s.contains("Tokyo") || s.contains("remote")), "two places at most");
    }

    #[test]
    fn remote_is_a_place_and_no_place_leaves_it_out() {
        let q = build_queries(&prefs(&["Rust Engineer"], &[], Remote::Remote), None).unwrap();
        assert_eq!(q[0], "\"Rust Engineer\" remote site:boards.greenhouse.io");
        let q = build_queries(&prefs(&["Rust Engineer"], &[], Remote::Onsite), None).unwrap();
        assert_eq!(q, [
            "\"Rust Engineer\" site:boards.greenhouse.io",
            "\"Rust Engineer\" site:jobs.lever.co",
            "\"Rust Engineer\" site:jobs.ashbyhq.com",
            "\"Rust Engineer\" jobs careers",
        ]);
    }

    #[test]
    fn quotes_and_line_breaks_cannot_break_out_of_the_phrase() {
        let q = build_queries(&prefs(&["Rust \"Engineer\"\n site:evil.test", "  "], &["Ber\"lin"], Remote::Any), None).unwrap();
        assert_eq!(q[0], "\"Rust Engineer site:evil.test\" Ber lin site:boards.greenhouse.io");
        assert_eq!(q.len(), 4, "the blank role is dropped");
    }

    #[test]
    fn without_wanted_roles_the_profile_roles_are_searched() {
        let mut cv = Cv::default();
        cv.basics.headline = "Platform Engineer".into();
        cv.experience.push(Experience { position: "Senior Backend Developer".into(), ..Experience::default() });
        let q = build_queries(&prefs(&[], &["Hanoi"], Remote::Any), Some(&cv)).unwrap();
        assert_eq!(q[0], "\"Senior Backend Developer\" Hanoi site:boards.greenhouse.io");
        assert_eq!(q[4], "\"Platform Engineer\" Hanoi site:boards.greenhouse.io");
    }

    #[test]
    fn no_roles_and_no_profile_is_the_no_profile_problem() {
        let err = build_queries(&prefs(&[], &["Hanoi"], Remote::Any), None).unwrap_err();
        assert_eq!(classify(&format!("{err:#}")), Problem::NoProfile);
        let err = build_queries(&prefs(&[], &[], Remote::Any), Some(&Cv::default())).unwrap_err();
        assert!(err.to_string().contains("Profile > Preferences"), "{err}");
    }
}
