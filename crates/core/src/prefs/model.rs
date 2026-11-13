//! The saved job-search preferences: what the user wants, what to avoid, and how much each
//! criterion counts. Every field is optional, so `Preferences::default()` is valid and discovery,
//! scoring and trends work before the preference chat has run.

use serde::{Deserialize, Serialize};

/// Where the user is willing to work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Remote {
    /// No preference.
    #[default]
    Any,
    Remote,
    Hybrid,
    Onsite,
}

impl Remote {
    /// Short label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Remote::Any => "Any",
            Remote::Remote => "Remote",
            Remote::Hybrid => "Hybrid",
            Remote::Onsite => "On-site",
        }
    }
}

/// How much each scoring criterion counts. The default (and a [`normalized`](Weights::normalized)
/// value) sums to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Weights {
    pub skills: u32,
    pub domain: u32,
    pub location: u32,
    pub salary: u32,
    pub seniority: u32,
}

impl Default for Weights {
    fn default() -> Self {
        Self { skills: 35, domain: 20, location: 15, salary: 15, seniority: 15 }
    }
}

impl Weights {
    /// Sum of all weights.
    pub fn sum(&self) -> u32 {
        self.skills.saturating_add(self.domain).saturating_add(self.location).saturating_add(self.salary).saturating_add(self.seniority)
    }

    /// Scaled so the weights sum to exactly 100, keeping their proportions (largest remainder
    /// rounding). All zeros fall back to the default.
    pub fn normalized(self) -> Self {
        let w = [self.skills, self.domain, self.location, self.salary, self.seniority].map(u64::from);
        let total: u64 = w.iter().sum();
        if total == 0 {
            return Self::default();
        }
        let mut out = w.map(|x| x * 100 / total);
        let missing = 100 - out.iter().sum::<u64>();
        let mut by_remainder = [0usize, 1, 2, 3, 4];
        by_remainder.sort_by_key(|&i| std::cmp::Reverse(w[i] * 100 % total));
        for &i in by_remainder.iter().take(missing as usize) {
            out[i] += 1;
        }
        let [skills, domain, location, salary, seniority] = out.map(|x| x as u32);
        Self { skills, domain, location, salary, seniority }
    }
}

/// What the user is looking for. Saved as one JSON document (`Store::save_preferences`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Target role titles ("Backend Engineer").
    pub roles: Vec<String>,
    /// Target levels ("senior", "staff").
    pub seniority: Vec<String>,
    /// Skills a job must ask for.
    pub must_skills: Vec<String>,
    /// Skills that make a job more attractive.
    pub nice_skills: Vec<String>,
    /// Skills the user does not want to work with.
    pub avoid_skills: Vec<String>,
    /// Business domains of interest ("fintech").
    pub domains: Vec<String>,
    pub avoid_domains: Vec<String>,
    /// Cities / countries.
    pub locations: Vec<String>,
    pub remote: Remote,
    /// Minimum salary, in `currency`.
    pub salary_min: Option<u64>,
    /// ISO currency code of `salary_min`.
    pub currency: Option<String>,
    pub company_size: Vec<String>,
    pub company_stage: Vec<String>,
    /// `Some(true)`: the user needs visa sponsorship.
    pub visa_needed: Option<bool>,
    /// Working languages.
    pub languages: Vec<String>,
    /// Companies to never show.
    pub exclude_companies: Vec<String>,
    /// Anything else the user said.
    pub notes: String,
    pub weights: Weights,
}

impl Preferences {
    /// A tidy copy: weights summing to 100, list entries trimmed, empty ones dropped and
    /// duplicates (ignoring case) removed, blank currency/notes cleared.
    pub fn normalized(mut self) -> Self {
        self.weights = self.weights.normalized();
        for list in [
            &mut self.roles,
            &mut self.seniority,
            &mut self.must_skills,
            &mut self.nice_skills,
            &mut self.avoid_skills,
            &mut self.domains,
            &mut self.avoid_domains,
            &mut self.locations,
            &mut self.company_size,
            &mut self.company_stage,
            &mut self.languages,
            &mut self.exclude_companies,
        ] {
            tidy(list);
        }
        self.currency = self.currency.map(|c| c.trim().to_uppercase()).filter(|c| !c.is_empty());
        self.notes = self.notes.trim().to_string();
        self
    }
}

fn tidy(list: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    *list = std::mem::take(list)
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid_and_sums_to_100() {
        let p = Preferences::default();
        assert_eq!(p.weights.sum(), 100);
        assert_eq!(p.remote, Remote::Any);
        assert_eq!(p.clone().normalized(), p, "a default needs no tidying");
    }

    #[test]
    fn weights_are_renormalised_to_100() {
        let w = Weights { skills: 1, domain: 1, location: 1, salary: 0, seniority: 0 }.normalized();
        assert_eq!(w.sum(), 100);
        assert_eq!((w.salary, w.seniority), (0, 0));
        assert!(w.skills >= 33 && w.domain >= 33 && w.location >= 33);

        let w = Weights { skills: 200, domain: 100, location: 0, salary: 0, seniority: 0 }.normalized();
        assert_eq!((w.skills, w.domain), (67, 33));

        let zero = Weights { skills: 0, domain: 0, location: 0, salary: 0, seniority: 0 };
        assert_eq!(zero.normalized(), Weights::default());
        let big = Weights { skills: u32::MAX, domain: u32::MAX, location: u32::MAX, salary: u32::MAX, seniority: u32::MAX };
        assert_eq!(big.normalized().sum(), 100, "no overflow");
    }

    #[test]
    fn partial_json_fills_defaults() {
        let p: Preferences = serde_json::from_str(r#"{"roles": ["SRE"], "remote": "hybrid", "weights": {"skills": 50}}"#).unwrap();
        assert_eq!(p.roles, ["SRE"]);
        assert_eq!(p.remote, Remote::Hybrid);
        assert_eq!(p.weights.skills, 50);
        assert_eq!(p.weights.domain, Weights::default().domain);
        assert!(p.must_skills.is_empty() && p.salary_min.is_none());
        let empty: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Preferences::default());
        assert!(serde_json::from_str::<Preferences>(r#"{"remote": "moon"}"#).is_err());
    }

    #[test]
    fn normalized_tidies_lists() {
        let p = Preferences {
            roles: vec![" Backend ".into(), "backend".into(), "".into(), "SRE".into()],
            currency: Some(" usd ".into()),
            notes: "  hi \n".into(),
            weights: Weights { skills: 1, domain: 1, location: 0, salary: 0, seniority: 0 },
            ..Default::default()
        }
        .normalized();
        assert_eq!(p.roles, ["Backend", "SRE"]);
        assert_eq!(p.currency.as_deref(), Some("USD"));
        assert_eq!(p.notes, "hi");
        assert_eq!(p.weights.sum(), 100);
        assert_eq!(Preferences { currency: Some("  ".into()), ..Default::default() }.normalized().currency, None);
    }
}
