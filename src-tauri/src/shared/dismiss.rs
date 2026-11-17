//! What "Dismiss" and "Restore" did, as both contexts report it: Leads dismiss and restore leads
//! (and restore jobs from its Dismissed list), Applications dismiss and restore jobs in CVs.

use jobpilot_engine::discover::promote::Dismissed;
use jobpilot_engine::discover::Restored;
use serde::Serialize;

/// What "Dismiss" did (the web app words it).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissedDto {
    /// Other leads of the company dismissed with it ("Don't show {company} again").
    pub also: usize,
    /// Other leads of the company kept because their CV is queued or being written.
    pub kept: usize,
    /// "Don't show {company} again" was asked for but could not be saved (why).
    pub not_excluded: Option<String>,
}

impl From<Dismissed> for DismissedDto {
    fn from(d: Dismissed) -> Self {
        Self {
            also: d.also,
            kept: d.kept,
            not_excluded: d.not_excluded,
        }
    }
}

/// What "Restore" did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredDto {
    /// The restored lead is hidden by the saved preferences (why): back, but not in the list
    /// until they let it pass. `None` when it is shown, and for a job from CVs.
    pub hidden: Option<String>,
    /// Its company is still excluded in the preferences: later searches keep leaving it out.
    pub excluded: bool,
}

impl From<Restored> for RestoredDto {
    fn from(r: Restored) -> Self {
        Self {
            hidden: r.hidden,
            excluded: r.excluded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismiss_and_restore_outcomes_travel_in_camel_case() {
        let dismissed = DismissedDto::from(Dismissed { also: 2, kept: 1, not_excluded: Some("disk full".into()) });
        assert_eq!(
            serde_json::to_value(&dismissed).unwrap(),
            serde_json::json!({"also": 2, "kept": 1, "notExcluded": "disk full"})
        );
        let restored = RestoredDto::from(Restored { hidden: Some("excluded company: Acme".into()), excluded: true });
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::json!({"hidden": "excluded company: Acme", "excluded": true})
        );
    }
}
