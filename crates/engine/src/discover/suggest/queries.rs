//! The web searches of "Suggest sources", built in code from the terms the user picked: roles,
//! business domains, skills, places and remote.
//!
//! ```text
//! "Rust Engineer" Berlin site:boards.greenhouse.io
//! "Fintech" Berlin site:boards.greenhouse.io
//! "Rust Engineer" Berlin site:jobs.lever.co
//! "Fintech" Berlin site:jobs.lever.co
//! ...
//! "Rust Engineer" jobs Berlin careers
//! ```
//!
//! [`SuggestTerms::available`] lists what can be picked: the preferences' target roles (without
//! any, the roles the profile shows: [`MarketFrame`], its visible positions and headline), domains,
//! skills (must, then nice to have), preferred locations, and remote when the user wants remote
//! work. The user picks values from it (remembered, see `Core::suggest_options`);
//! [`SuggestTerms::default_pick`] is the pick without one: every role, the first places and remote.
//! Each picked role, domain or skill is searched on its own ([`build_queries_for`]).

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::pipeline::cv::Cv;
use crate::prefs::{Preferences, Remote};
use crate::profile::NO_PROFILE;
use crate::trends::frame::MarketFrame;

/// Most searches of one run.
pub const MAX_QUERIES: usize = 8;
/// Most places searched for one term.
const MAX_PLACES: usize = 2;
/// Longest term or place phrase put into a query.
const MAX_PHRASE_CHARS: usize = 60;
/// The job boards searched by name (each `site:` query finds that board's companies).
const BOARD_SITES: [&str; 3] = ["boards.greenhouse.io", "jobs.lever.co", "jobs.ashbyhq.com"];

/// The error when there is a profile but neither it nor the preferences name a role.
pub const NO_ROLES: &str = "no roles to search for: add the roles you want in Profile > Preferences";
/// The error when the pick holds no role, domain or skill.
pub const NO_TERMS: &str = "nothing to search for: pick at least one role, domain or skill";

/// What a search is built from: the values offered ([`SuggestTerms::available`]) or the ones the
/// user picked among them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuggestTerms {
    pub roles: Vec<String>,
    pub domains: Vec<String>,
    pub skills: Vec<String>,
    pub places: Vec<String>,
    /// Search remote jobs too (a place of its own).
    pub remote: bool,
}

impl SuggestTerms {
    /// What the user can pick from: the preferences' roles (without any, the profile's roles when
    /// there is a profile `cv`), domains, must then nice-to-have skills, locations, and remote when
    /// the user wants remote work. Each value cleaned for a query, blanks and repeats (ignoring
    /// case) dropped.
    pub fn available(prefs: &Preferences, cv: Option<&Cv>) -> SuggestTerms {
        let mut roles = cleaned(&prefs.roles);
        if roles.is_empty() {
            if let Some(cv) = cv {
                roles = cleaned(&MarketFrame::new(Some(cv), prefs).roles);
            }
        }
        let skills: Vec<String> = prefs.must_skills.iter().chain(&prefs.nice_skills).cloned().collect();
        SuggestTerms {
            roles,
            domains: cleaned(&prefs.domains),
            skills: cleaned(&skills),
            places: cleaned(&prefs.locations),
            remote: prefs.remote == Remote::Remote,
        }
    }

    /// The pick when the user made none: every role, the first places, remote when available; no
    /// domain, no skill (the search as it was before the pick existed).
    pub fn default_pick(available: &SuggestTerms) -> SuggestTerms {
        SuggestTerms {
            roles: available.roles.clone(),
            domains: Vec::new(),
            skills: Vec::new(),
            places: available.places.iter().take(MAX_PLACES).cloned().collect(),
            remote: available.remote,
        }
    }

    /// This pick without the values `available` no longer offers (compared ignoring case; the
    /// available spelling is kept), and remote only when it is offered.
    pub fn within(&self, available: &SuggestTerms) -> SuggestTerms {
        SuggestTerms {
            roles: kept(&self.roles, &available.roles),
            domains: kept(&self.domains, &available.domains),
            skills: kept(&self.skills, &available.skills),
            places: kept(&self.places, &available.places),
            remote: self.remote && available.remote,
        }
    }
}

/// The values cleaned by [`phrase`], blanks and repeats (ignoring case) dropped.
fn cleaned(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values.iter().map(|v| phrase(v)).filter(|v| !v.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&value)) {
            out.push(value);
        }
    }
    out
}

/// The `available` values that `picked` names (ignoring case), each once, in the picked order.
fn kept(picked: &[String], available: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in picked.iter().map(|p| phrase(p)) {
        if let Some(a) = available.iter().find(|a| a.eq_ignore_ascii_case(&p)) {
            if !out.contains(a) {
                out.push(a.clone());
            }
        }
    }
    out
}

/// Up to [`MAX_QUERIES`] searches for the default pick of the preferences (and the profile's roles
/// when they name none). `cv` is the stored profile, if there is one. Without roles in the
/// preferences and without a profile, the [`NO_PROFILE`] error (`health::classify` maps it to
/// `Problem::NoProfile`); with a profile that shows no role either, [`NO_ROLES`].
pub fn build_queries(prefs: &Preferences, cv: Option<&Cv>) -> Result<Vec<String>> {
    let available = SuggestTerms::available(prefs, cv);
    if available.roles.is_empty() {
        if cv.is_none() {
            bail!(NO_PROFILE);
        }
        bail!(NO_ROLES);
    }
    build_queries_for(&SuggestTerms::default_pick(&available))
}

/// Up to [`MAX_QUERIES`] searches for a pick. The terms are its roles, then domains, then skills,
/// each searched on its own; the places are its first [`MAX_PLACES`] places, then `remote` when
/// picked and there is room (none: no place). Each term and place gives one query per board in
/// [`BOARD_SITES`] and one for company careers pages; the queries go round-robin (the first form of
/// every term in the first place, then in the second place, then the second form, ...), so every
/// term gets a search before the cap (up to [`MAX_QUERIES`] terms).
/// [`NO_TERMS`] when the pick has no role, domain or skill.
pub fn build_queries_for(terms: &SuggestTerms) -> Result<Vec<String>> {
    let words: Vec<String> = terms.roles.iter().chain(&terms.domains).chain(&terms.skills).cloned().collect();
    let words = cleaned(&words);
    if words.is_empty() {
        bail!(NO_TERMS);
    }
    let mut places: Vec<String> = cleaned(&terms.places).into_iter().take(MAX_PLACES).collect();
    if places.len() < MAX_PLACES && terms.remote && !places.iter().any(|p| p.eq_ignore_ascii_case("remote")) {
        places.push("remote".into());
    }
    if places.is_empty() {
        places.push(String::new());
    }

    let mut pairs: Vec<Vec<String>> = Vec::new();
    // Place by place, so each term gets its first search before a second place does.
    for place in &places {
        for word in &words {
            let quoted = format!("\"{word}\"");
            let mut forms: Vec<String> = BOARD_SITES.iter().map(|site| join(&[&quoted, place, &format!("site:{site}")])).collect();
            forms.push(join(&[&quoted, "jobs", place, "careers"]));
            pairs.push(forms);
        }
    }
    let rounds = BOARD_SITES.len() + 1;
    let mut out: Vec<String> = Vec::new();
    for round in 0..rounds {
        for forms in &pairs {
            let Some(query) = forms.get(round) else { continue };
            if !out.contains(query) {
                out.push(query.clone());
            }
            if out.len() == MAX_QUERIES {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// A term or place as it goes into a query: one line, no quotes, at most [`MAX_PHRASE_CHARS`].
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
        assert_eq!(
            q[..6],
            [
                "\"Backend Engineer\" Hanoi site:boards.greenhouse.io",
                "\"Data Engineer\" Hanoi site:boards.greenhouse.io",
                "\"SRE\" Hanoi site:boards.greenhouse.io",
                "\"Backend Engineer\" Singapore site:boards.greenhouse.io",
                "\"Data Engineer\" Singapore site:boards.greenhouse.io",
                "\"SRE\" Singapore site:boards.greenhouse.io",
            ],
            "every role, then every role in the next place, before any second form"
        );
        assert_eq!(q[6], "\"Backend Engineer\" Hanoi site:jobs.lever.co");
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
        assert_eq!(q[1], "\"Platform Engineer\" Hanoi site:boards.greenhouse.io");
    }

    #[test]
    fn no_roles_and_no_profile_is_the_no_profile_problem() {
        let err = build_queries(&prefs(&[], &["Hanoi"], Remote::Any), None).unwrap_err();
        assert_eq!(classify(&format!("{err:#}")), Problem::NoProfile);
        let err = build_queries(&prefs(&[], &[], Remote::Any), Some(&Cv::default())).unwrap_err();
        assert!(err.to_string().contains("Profile > Preferences"), "{err}");
    }

    fn terms(roles: &[&str], domains: &[&str], skills: &[&str], places: &[&str], remote: bool) -> SuggestTerms {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        SuggestTerms { roles: v(roles), domains: v(domains), skills: v(skills), places: v(places), remote }
    }

    #[test]
    fn a_domain_or_a_skill_alone_is_searched_like_a_role() {
        let q = build_queries_for(&terms(&[], &["Fintech"], &[], &["Berlin"], false)).unwrap();
        assert_eq!(
            q,
            [
                "\"Fintech\" Berlin site:boards.greenhouse.io",
                "\"Fintech\" Berlin site:jobs.lever.co",
                "\"Fintech\" Berlin site:jobs.ashbyhq.com",
                "\"Fintech\" jobs Berlin careers",
            ]
        );
        let q = build_queries_for(&terms(&[], &[], &["Rust"], &[], true)).unwrap();
        assert_eq!(q[0], "\"Rust\" remote site:boards.greenhouse.io");
        assert_eq!(q[3], "\"Rust\" jobs remote careers");
    }

    #[test]
    fn many_terms_under_the_cap_each_get_a_search() {
        let t = terms(&["Backend Engineer", "SRE"], &["Fintech", "Health"], &["Rust", "Go"], &["Berlin"], true);
        let q = build_queries_for(&t).unwrap();
        assert_eq!(q.len(), MAX_QUERIES);
        for term in ["Backend Engineer", "SRE", "Fintech", "Health", "Rust", "Go"] {
            assert!(q.iter().any(|s| s.starts_with(&format!("\"{term}\" "))), "{term} is searched: {q:?}");
        }
        assert_eq!(q[0], "\"Backend Engineer\" Berlin site:boards.greenhouse.io");
        assert_eq!(q[1], "\"SRE\" Berlin site:boards.greenhouse.io");
        assert_eq!(q[6], "\"Backend Engineer\" remote site:boards.greenhouse.io");
    }

    #[test]
    fn a_pick_without_a_role_domain_or_skill_is_refused() {
        let err = build_queries_for(&terms(&[], &[], &[], &["Berlin"], true)).unwrap_err();
        assert_eq!(err.to_string(), NO_TERMS);
        let err = build_queries_for(&terms(&["  "], &["\""], &[], &[], false)).unwrap_err();
        assert_eq!(err.to_string(), NO_TERMS, "blank terms do not count");
        assert_eq!(classify(NO_TERMS), Problem::Other);
    }

    #[test]
    fn quotes_and_line_breaks_cannot_break_out_of_a_domain() {
        let q = build_queries_for(&terms(&[], &["Fin\"tech\"\n site:evil.test"], &[], &[], false)).unwrap();
        assert_eq!(q[0], "\"Fin tech site:evil.test\" site:boards.greenhouse.io");
    }

    #[test]
    fn the_available_terms_come_from_the_preferences() {
        let p = Preferences {
            roles: vec!["Rust Engineer".into(), "rust engineer".into()],
            domains: vec!["Fintech".into(), " ".into()],
            must_skills: vec!["Rust".into(), "Tokio".into()],
            nice_skills: vec!["rust".into(), "Kafka".into()],
            locations: vec!["Berlin".into(), "Hanoi".into(), "Tokyo".into()],
            remote: Remote::Remote,
            ..Preferences::default()
        };
        let a = SuggestTerms::available(&p, None);
        assert_eq!(a, terms(&["Rust Engineer"], &["Fintech"], &["Rust", "Tokio", "Kafka"], &["Berlin", "Hanoi", "Tokyo"], true));
        assert_eq!(SuggestTerms::default_pick(&a), terms(&["Rust Engineer"], &[], &[], &["Berlin", "Hanoi"], true));
        assert_eq!(build_queries_for(&SuggestTerms::default_pick(&a)).unwrap(), build_queries(&p, None).unwrap());

        let mut cv = Cv::default();
        cv.basics.headline = "Platform Engineer".into();
        let a = SuggestTerms::available(&Preferences::default(), Some(&cv));
        assert_eq!(a.roles, ["Platform Engineer"], "the profile's roles without wanted ones");
        assert!(!a.remote);
    }

    #[test]
    fn within_drops_what_is_no_longer_offered() {
        let available = terms(&["Rust Engineer"], &["Fintech"], &["Rust"], &["Berlin"], false);
        let picked = terms(&["rust engineer", "Go Engineer"], &["Fintech", "Gaming"], &[], &["Berlin", "Tokyo"], true);
        assert_eq!(picked.within(&available), terms(&["Rust Engineer"], &["Fintech"], &[], &["Berlin"], false));
        assert_eq!(SuggestTerms::default().within(&available), SuggestTerms::default());
    }
}
