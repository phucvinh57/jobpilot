//! Ideas for the preferences screen: skills from the master profile, and the locations and business
//! domains of the jobs found so far. Each list is something the user could add to their preferences
//! with one click, so it never repeats what the saved preferences already say.
//!
//! ```text
//! let s = core.preference_suggestions()?;      // one local read, no network and no model call
//! for skill in &s.skills { .. }                // "Rust", "Go", ... (profile order)
//! for place in &s.locations { .. }             // most common among the leads first, "Remote" last
//! ```

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use super::Preferences;
use crate::pipeline::cv::Cv;
use crate::pipeline::Insight;
use crate::store::{Lead, LeadState};
use crate::Core;

/// Longest list of suggestions.
pub const MAX_SUGGESTIONS: usize = 12;

/// What the preferences screen can offer, none of it already in the saved preferences.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrefSuggestions {
    /// Skills in the master profile (visible groups only), in profile order.
    pub skills: Vec<String>,
    /// Locations of the leads, the most common first, plus "Remote".
    pub locations: Vec<String>,
    /// Business domains of the leads' briefs, the most common first.
    pub domains: Vec<String>,
}

impl Core {
    /// Suggestions for the preferences screen: skills from `profile.yaml`, locations and domains
    /// from the leads that are in the queue or were prepared (dismissed leads say what the user
    /// does not want). Each list has no repeats (ignoring case), at most [`MAX_SUGGESTIONS`]
    /// entries, and leaves out what the saved preferences already name. A profile that cannot be
    /// read only means no skills.
    pub fn preference_suggestions(&self) -> Result<PrefSuggestions> {
        let prefs = self.store.preferences()?;
        let mut leads = self.store.list_leads(LeadState::New)?;
        leads.extend(self.store.list_leads(LeadState::Promoted)?);
        Ok(suggest(&prefs, profile_skills(self), &leads))
    }
}

impl Core {
    /// The visible skill names in `profile.yaml` (hidden groups left out); empty when the profile
    /// cannot be read (logged).
    pub fn profile_skills(&self) -> Vec<String> {
        profile_skills(self)
    }
}

/// The skills of the visible groups of the master profile.
fn profile_skills(core: &Core) -> Vec<String> {
    let path = core.profile_path();
    let cv = match std::fs::read_to_string(&path).map_err(anyhow::Error::from).and_then(|text| Cv::parse(&text)) {
        Ok(cv) => cv,
        Err(e) => {
            tracing::warn!("no skill suggestions: the profile cannot be read: {e:#}");
            return Vec::new();
        }
    };
    cv.skills.into_iter().filter(|g| !g.hidden).flat_map(|g| g.items).collect()
}

fn suggest(prefs: &Preferences, skills: Vec<String>, leads: &[Lead]) -> PrefSuggestions {
    let have_skills = [&prefs.must_skills, &prefs.nice_skills, &prefs.avoid_skills];
    let have_places = [&prefs.locations];
    let have_domains = [&prefs.domains, &prefs.avoid_domains];

    let skills = pick(skills, &have_skills, None);

    let places = most_common(leads.iter().map(|l| l.location.as_str()));
    let locations = pick(places, &have_places, Some("Remote"));

    let domains = most_common(
        leads.iter().filter_map(|l| l.insight_json.as_deref()).filter_map(Insight::from_json).map(|i| i.domain),
    );
    let domains = pick(domains, &have_domains, None);

    PrefSuggestions { skills, locations, domains }
}

/// One line, single spaces.
fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The distinct values (ignoring case, first spelling kept), the most common first; ties keep the
/// order they were first seen in. Empty values are dropped.
fn most_common<S: AsRef<str>>(values: impl Iterator<Item = S>) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for value in values {
        let value = tidy(value.as_ref());
        if value.is_empty() {
            continue;
        }
        let n = counts.entry(value.to_lowercase()).or_insert(0);
        if *n == 0 {
            order.push(value);
        }
        *n += 1;
    }
    order.sort_by_key(|v| std::cmp::Reverse(counts[&v.to_lowercase()]));
    order
}

/// `candidates` without repeats (ignoring case), without anything in `taken`, at most
/// [`MAX_SUGGESTIONS`]. `always` is added last when it is not there already (and not taken), and
/// is kept even when the list is full.
fn pick(candidates: Vec<String>, taken: &[&Vec<String>], always: Option<&str>) -> Vec<String> {
    let mut seen: HashSet<String> = taken.iter().flat_map(|list| list.iter()).map(|s| tidy(s).to_lowercase()).collect();
    let mut out: Vec<String> = candidates
        .into_iter()
        .map(|s| tidy(&s))
        .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
        .collect();
    if let Some(extra) = always {
        out.truncate(MAX_SUGGESTIONS - usize::from(!seen.contains(&extra.to_lowercase())));
        if seen.insert(extra.to_lowercase()) {
            out.push(extra.to_string());
        }
    } else {
        out.truncate(MAX_SUGGESTIONS);
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::NewLead;
    use crate::{Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        let profile = dir.path().join("profile.yaml");
        std::fs::write(
            &profile,
            "basics: {name: Ada}\nskills:\n  - {category: Languages, items: [Rust, Go, rust]}\n  - {category: Cloud, items: [AWS]}\n  - {category: Old, items: [COBOL], hidden: true}\n",
        )
        .unwrap();
        core.set_profile_path(&profile).unwrap();
        (dir, core)
    }

    fn lead(core: &Core, n: u32, location: &str, domain: Option<&str>) -> Lead {
        core.store
            .add_lead(&NewLead {
                url: format!("https://a.test/{n}"),
                company: format!("Co{n}"),
                role: "Engineer".into(),
                location: location.into(),
                insight_json: domain.map(|d| json!({"domain": d}).to_string()),
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn nothing_known_gives_skills_and_remote_only() {
        let (_dir, core) = core();
        let s = core.preference_suggestions().unwrap();
        assert_eq!(s.skills, ["Rust", "Go", "AWS"], "profile order, no repeats, hidden groups left out");
        assert_eq!(s.locations, ["Remote"]);
        assert!(s.domains.is_empty());
    }

    #[test]
    fn locations_and_domains_come_from_the_leads_most_common_first() {
        let (_dir, core) = core();
        lead(&core, 1, "Berlin", Some("Fintech"));
        lead(&core, 2, " ho chi minh  city ", Some("Robotics"));
        lead(&core, 3, "Ho Chi Minh City", Some("fintech"));
        lead(&core, 4, "", None);
        lead(&core, 5, "Remote", Some("Robotics"));
        lead(&core, 6, "remote", Some(""));
        lead(&core, 7, "Remote", Some("Fintech"));
        let s = core.preference_suggestions().unwrap();
        assert_eq!(s.locations, ["Remote", "Ho Chi Minh City", "Berlin"], "3 remote, 2 in HCMC, 1 in Berlin; Remote not repeated; the spelling of the newest lead wins");
        assert_eq!(s.domains, ["Fintech", "Robotics"]);
    }

    #[test]
    fn dismissed_leads_and_unreadable_briefs_give_nothing() {
        let (_dir, core) = core();
        let gone = lead(&core, 1, "Paris", Some("Gaming"));
        core.store.dismiss_lead(gone.id, "no").unwrap();
        core.store
            .add_lead(&NewLead { url: "https://a.test/2".into(), location: "Oslo".into(), insight_json: Some("not json".into()), ..Default::default() })
            .unwrap();
        let s = core.preference_suggestions().unwrap();
        assert_eq!(s.locations, ["Oslo", "Remote"]);
        assert!(s.domains.is_empty());
    }

    #[test]
    fn what_the_preferences_already_say_is_left_out() {
        let (_dir, core) = core();
        lead(&core, 1, "Berlin", Some("Fintech"));
        lead(&core, 2, "Paris", Some("Robotics"));
        core.save_preferences(Preferences {
            must_skills: vec!["rust".into()],
            nice_skills: vec!["AWS".into()],
            locations: vec!["berlin".into(), "REMOTE".into()],
            domains: vec!["FINTECH".into()],
            ..Default::default()
        })
        .unwrap();
        let s = core.preference_suggestions().unwrap();
        assert_eq!(s.skills, ["Go"]);
        assert_eq!(s.locations, ["Paris"], "Remote is not offered when it is saved already");
        assert_eq!(s.domains, ["Robotics"]);

        core.save_preferences(Preferences { avoid_skills: vec!["Go".into()], avoid_domains: vec!["Robotics".into()], ..Default::default() }).unwrap();
        let s = core.preference_suggestions().unwrap();
        assert_eq!(s.skills, ["Rust", "AWS"], "a skill to avoid is not offered either");
        assert_eq!(s.domains, ["Fintech"]);
    }

    #[test]
    fn lists_are_capped_and_remote_survives_the_cap() {
        let (_dir, core) = core();
        for n in 0..20 {
            lead(&core, n, &format!("City {n}"), Some(&format!("Domain {n}")));
        }
        let s = core.preference_suggestions().unwrap();
        assert_eq!(s.locations.len(), MAX_SUGGESTIONS);
        assert_eq!(s.locations.last().map(String::as_str), Some("Remote"));
        assert_eq!(s.locations[0], "City 19", "equal counts keep the order of the leads (newest first)");
        assert_eq!(s.domains.len(), MAX_SUGGESTIONS);

        let many: Vec<String> = (0..30).map(|n| format!("Skill {n}")).collect();
        assert_eq!(pick(many, &[], None).len(), MAX_SUGGESTIONS);
    }

    #[test]
    fn a_missing_or_broken_profile_only_means_no_skills() {
        let (dir, core) = core();
        core.set_profile_path(&dir.path().join("missing.yaml")).unwrap();
        assert!(core.preference_suggestions().unwrap().skills.is_empty());
        let broken = dir.path().join("broken.yaml");
        std::fs::write(&broken, "skills: [unclosed").unwrap();
        core.set_profile_path(&broken).unwrap();
        let s = core.preference_suggestions().unwrap();
        assert!(s.skills.is_empty());
        assert_eq!(s.locations, ["Remote"]);
    }
}
