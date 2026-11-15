//! The market frame: what decides which part of the market a trend run looks at. It comes from the
//! user's background (the master profile: positions, headline, skills) and place (the preferred
//! locations and work mode), never from their wishes (wanted or avoided skills, wanted domains,
//! target roles). Wishes are checked against the market at the end of the digest, so they cannot
//! shape what is collected or counted (design: `docs/plans/P15-market-scan.md`).

use std::collections::HashSet;
use std::path::Path;

use crate::engine::discover::score::compact_profile;
use crate::engine::discover::source::{mentions_lowered, words, ROLE_NOISE};
use crate::engine::pipeline::cv::Cv;
use crate::engine::prefs::{Preferences, Remote};

/// Role phrases kept from the profile (the newest positions first).
const MAX_ROLES: usize = 3;
/// Longest role phrase kept.
const MAX_ROLE_CHARS: usize = 60;

/// The part of the market a run is about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarketFrame {
    /// Role phrases from the profile (visible positions, newest first, then the headline), at most
    /// [`MAX_ROLES`]: "Backend Engineer". Used to build report searches.
    pub roles: Vec<String>,
    /// The meaningful words of [`roles`](Self::roles) ("backend", "engineer"): a posting that names
    /// one is in the user's field.
    pub role_words: Vec<String>,
    /// Preferred locations, as the user wrote them.
    pub locations: Vec<String>,
    /// The work mode allows remote work (`Any`, `Remote` or `Hybrid`).
    pub remote_ok: bool,
    /// The visible skills of the profile.
    pub skills: Vec<String>,
    /// The compact profile (no contact details) the digest prompt shows as the user's background;
    /// empty when the profile cannot be read.
    pub background: String,
}

impl MarketFrame {
    /// The frame from a parsed profile (`None` when it cannot be read) and the preferences. Reads
    /// only `locations` and `remote` of the preferences.
    pub fn new(cv: Option<&Cv>, prefs: &Preferences) -> Self {
        let mut roles: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        if let Some(cv) = cv {
            let positions = cv.experience.iter().filter(|e| !e.hidden).map(|e| e.position.as_str());
            for phrase in positions.chain(std::iter::once(cv.basics.headline.as_str())) {
                let phrase = one_line(phrase);
                if phrase.is_empty() || phrase.chars().count() > MAX_ROLE_CHARS || role_words_of(&phrase).is_empty() {
                    continue;
                }
                if seen.insert(phrase.to_lowercase()) {
                    roles.push(phrase);
                }
                if roles.len() == MAX_ROLES {
                    break;
                }
            }
        }
        let mut role_words: Vec<String> = Vec::new();
        for word in roles.iter().flat_map(|r| role_words_of(r)) {
            if !role_words.contains(&word) {
                role_words.push(word);
            }
        }
        let locations = prefs.locations.iter().map(|l| one_line(l)).filter(|l| !l.is_empty()).collect();
        let skills = cv
            .map(|cv| cv.skills.iter().filter(|g| !g.hidden).flat_map(|g| g.items.iter().map(|s| one_line(s))).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        let background = cv.map(compact_profile).unwrap_or_default();
        Self { roles, role_words, locations, remote_ok: prefs.remote != Remote::Onsite, skills, background }
    }

    /// The frame from the profile at `profile` (logged and left without background when it cannot
    /// be read) and the preferences.
    pub fn load(profile: Option<&Path>, prefs: &Preferences) -> Self {
        let cv = profile.and_then(|path| match std::fs::read_to_string(path).map_err(anyhow::Error::from).and_then(|t| Cv::parse(&t)) {
            Ok(cv) => Some(cv),
            Err(e) => {
                tracing::warn!("the market frame has no background: the profile cannot be read: {e:#}");
                None
            }
        });
        Self::new(cv.as_ref(), prefs)
    }

    /// Does a posting (lower case) belong to the user's place: it names a preferred location, or
    /// it offers remote work and the work mode allows it? Without locations and with remote not
    /// allowed nothing is local.
    pub fn is_local(&self, lowered: &str) -> bool {
        self.locations.iter().any(|l| mentions_lowered(lowered, l))
            || (self.remote_ok && ["remote", "work from home", "wfh"].iter().any(|k| mentions_lowered(lowered, k)))
    }

    /// Is there a place to scope by?
    pub fn has_place(&self) -> bool {
        !self.locations.is_empty() || self.remote_ok
    }

    /// Does a posting (lower case) belong to the user's field: it names one of the role words? No
    /// role words means every posting does.
    pub fn in_field(&self, lowered: &str) -> bool {
        self.role_words.is_empty() || self.role_words.iter().any(|w| mentions_lowered(lowered, w))
    }

    /// Is `skill` one of the profile's skills (ignoring case)?
    pub fn has_skill(&self, skill: &str) -> bool {
        self.skills.iter().any(|s| s.eq_ignore_ascii_case(skill.trim()))
    }

    /// How the place reads in the view and the prompt: "Ho Chi Minh City, Singapore or remote".
    pub fn place_label(&self) -> String {
        let mut parts = self.locations.clone();
        if self.remote_ok {
            parts.push("remote".into());
        }
        match parts.len() {
            0 => String::new(),
            1 => parts.remove(0),
            n => format!("{} or {}", parts[..n - 1].join(", "), parts[n - 1]),
        }
    }
}

/// The words of a role phrase that say what the job is (not how senior it is).
fn role_words_of(phrase: &str) -> Vec<String> {
    words(phrase).into_iter().filter(|w| w.chars().count() >= 3 && !ROLE_NOISE.contains(&w.as_str())).collect()
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pipeline::cv::{Experience, SkillGroup};

    fn cv() -> Cv {
        let mut cv = Cv::default();
        cv.basics.headline = "Backend engineer".into();
        cv.basics.email = "me@example.com".into();
        cv.experience = vec![
            Experience { position: "Senior Software Engineer".into(), company: "Acme".into(), ..Experience::default() },
            Experience { position: "Hidden Role".into(), hidden: true, ..Experience::default() },
            Experience { position: "Data Analyst".into(), ..Experience::default() },
        ];
        cv.skills = vec![
            SkillGroup { category: "Languages".into(), items: vec!["Rust".into(), "Go".into()], hidden: false },
            SkillGroup { category: "Old".into(), items: vec!["Perl".into()], hidden: true },
        ];
        cv
    }

    fn prefs() -> Preferences {
        Preferences {
            locations: vec!["Ho Chi Minh City".into(), " Singapore ".into()],
            remote: Remote::Remote,
            must_skills: vec!["Kotlin".into()],
            nice_skills: vec!["Elixir".into()],
            avoid_skills: vec!["PHP".into()],
            domains: vec!["fintech".into()],
            roles: vec!["Product Manager".into()],
            ..Preferences::default()
        }
    }

    #[test]
    fn the_frame_comes_from_the_profile_and_the_place() {
        let frame = MarketFrame::new(Some(&cv()), &prefs());
        assert_eq!(frame.roles, ["Senior Software Engineer", "Data Analyst", "Backend engineer"]);
        assert_eq!(frame.role_words, ["software", "engineer", "data", "analyst", "backend"]);
        assert_eq!(frame.locations, ["Ho Chi Minh City", "Singapore"]);
        assert!(frame.remote_ok);
        assert_eq!(frame.skills, ["Rust", "Go"]);
        assert!(frame.background.contains("Rust"));
        assert!(!frame.background.contains("me@example.com"));
        assert_eq!(frame.place_label(), "Ho Chi Minh City, Singapore or remote");
    }

    #[test]
    fn wishes_never_change_the_frame() {
        let plain = Preferences { locations: prefs().locations, remote: Remote::Remote, ..Preferences::default() };
        assert_eq!(MarketFrame::new(Some(&cv()), &prefs()), MarketFrame::new(Some(&cv()), &plain));
    }

    #[test]
    fn local_and_field_matching() {
        let frame = MarketFrame::new(Some(&cv()), &prefs());
        assert!(frame.is_local("acme | backend | singapore | onsite"));
        assert!(frame.is_local("acme | remote (apac)"));
        assert!(!frame.is_local("acme | berlin | onsite"));
        assert!(frame.in_field("hiring a software engineer"));
        assert!(!frame.in_field("hiring an account executive"));
        assert!(frame.has_skill(" rust"));
        assert!(!frame.has_skill("Perl"));

        let onsite = MarketFrame::new(None, &Preferences { remote: Remote::Onsite, ..Preferences::default() });
        assert!(!onsite.has_place());
        assert!(!onsite.is_local("remote ok"));
        assert!(onsite.in_field("anything"), "no profile: every posting is in the field");
        assert_eq!(onsite.place_label(), "");
    }
}
