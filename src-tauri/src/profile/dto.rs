//! Read models of this context (serde, camelCase). Core types stay free of serde and Tauri.

use crate::engine::prefs::chat::{MAX_LIST_ITEMS, MAX_NOTES_CHARS};
use crate::engine::prefs::{PrefSuggestions, Preferences};
use serde::Serialize;

use super::rules::{job_type_key, visible_suggestions, Field, MAX_ITEM_CHARS};

/// One tag field of the card: Can work from, Work locations, Interested domains, Skills.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFieldDto {
    /// `work_from` / `locations` / `domains` / `skills`: what the web app sends back to add or
    /// remove an entry.
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    /// A muted line under the field that says what it does (only where one is needed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'static str>,
    pub items: Vec<String>,
    /// Entries that could be added with one click (none of them is in the preferences already).
    pub suggestions: Vec<String>,
}

/// Limits the screen needs to decide what can be saved (the core enforces them again).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsDto {
    pub max_list_items: usize,
    pub max_item_chars: usize,
    pub max_notes_chars: usize,
}

/// Everything the card shows, read in one go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDto {
    /// `any` / `remote` / `hybrid` / `onsite`.
    pub job_type: &'static str,
    pub fields: Vec<ListFieldDto>,
    /// The user's own words, as saved.
    pub notes: String,
    pub can_undo: bool,
    pub limits: LimitsDto,
}

impl ProfileDto {
    /// The card around `prefs`.
    pub fn from_parts(prefs: &Preferences, suggestions: &PrefSuggestions, can_undo: bool) -> Self {
        let fields = Field::ALL
            .iter()
            .map(|&field| ListFieldDto {
                key: field.key(),
                label: field.label(),
                placeholder: field.placeholder(),
                hint: field.hint(),
                items: field.list(prefs).clone(),
                suggestions: visible_suggestions(field, prefs, field.suggestions(suggestions)),
            })
            .collect();
        Self {
            job_type: job_type_key(prefs.remote),
            fields,
            notes: prefs.notes.clone(),
            can_undo,
            limits: LimitsDto {
                max_list_items: MAX_LIST_ITEMS,
                max_item_chars: MAX_ITEM_CHARS,
                max_notes_chars: MAX_NOTES_CHARS,
            },
        }
    }
}

// ------------------------------------------------------------------ About you

/// One document the profile is read from. Never its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutSourceDto {
    pub id: i64,
    /// File name or the note's title.
    pub name: String,
    /// `file` / `text`.
    pub kind: String,
    /// Characters kept.
    pub chars: usize,
    /// The file had more text than was kept.
    pub truncated: bool,
}

/// One role of the summary: "Position — Company · dates".
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutRoleDto {
    pub position: String,
    pub company: String,
    /// `start – end`, one side, or empty.
    pub dates: String,
}

/// One degree of the summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutEducationDto {
    pub degree: String,
    pub institution: String,
    pub dates: String,
}

/// What was understood from the documents, read-only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutSummaryDto {
    pub name: String,
    pub headline: String,
    pub roles: Vec<AboutRoleDto>,
    pub education: Vec<AboutEducationDto>,
    /// Every skill once, in profile order.
    pub skills: Vec<String>,
    /// `Language (fluency)`.
    pub languages: Vec<String>,
    /// Project names.
    pub projects: Vec<String>,
    /// Certification names.
    pub certifications: Vec<String>,
}

/// Size limits of the documents (the core enforces them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutLimitsDto {
    /// Per document.
    pub max_chars: usize,
    /// All documents together.
    pub max_total_chars: usize,
}

/// Profile > About you: the documents, where the reading stands and what was understood.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutDto {
    pub sources: Vec<AboutSourceDto>,
    /// `empty` (no documents) / `unread` (documents, no profile yet) / `stale` (the documents
    /// changed since the read) / `ready`.
    pub status: &'static str,
    pub summary: Option<AboutSummaryDto>,
    /// Facts found in the documents that could not be traced, so left out.
    pub left_out: Vec<String>,
    /// When the profile was read (RFC 3339).
    pub built_at: Option<String>,
    pub limits: AboutLimitsDto,
}

/// The answer of an add: the card, and why nothing was added when the user should be told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDto {
    pub profile: ProfileDto,
    pub message: Option<String>,
}

/// The answer of reading the notes: the card as the model's merge left it, and the muted line
/// under the notes (none after a clean read).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesReadDto {
    pub profile: ProfileDto,
    pub line: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::engine::prefs::{PrefSuggestions, Preferences, Remote};

    use super::ProfileDto;

    #[test]
    fn the_card_lists_every_field_with_the_suggestions_that_can_still_be_added() {
        let prefs = Preferences {
            work_from: vec!["Vietnam".into()],
            locations: vec!["Hanoi".into()],
            must_skills: vec!["Rust".into()],
            avoid_skills: vec!["PHP".into()],
            remote: Remote::Hybrid,
            notes: "Senior only".into(),
            ..Default::default()
        };
        let suggestions = PrefSuggestions {
            skills: vec!["rust".into(), "PHP".into(), "Go".into()],
            locations: vec!["Hanoi".into(), "Remote".into()],
            domains: vec![],
            work_from: vec!["vietnam".into(), "Thailand".into()],
        };
        let dto = ProfileDto::from_parts(&prefs, &suggestions, true);

        assert_eq!(dto.job_type, "hybrid");
        assert_eq!(dto.notes, "Senior only");
        assert!(dto.can_undo);

        let keys: Vec<_> = dto.fields.iter().map(|f| f.key).collect();
        assert_eq!(
            keys,
            ["work_from", "locations", "domains", "skills"],
            "the order of the card"
        );
        assert_eq!(dto.fields[0].label, "Can work from");
        assert_eq!(dto.fields[0].items, ["Vietnam"]);
        assert_eq!(dto.fields[0].suggestions, ["Thailand"]);
        assert!(dto.fields[0].hint.is_some_and(|h| h.contains("Hard limit")));
        assert!(
            dto.fields[1..].iter().all(|f| f.hint.is_none()),
            "only Can work from has a hint"
        );
        assert_eq!(dto.fields[1].items, ["Hanoi"]);
        assert_eq!(
            dto.fields[1].suggestions,
            ["Remote"],
            "what is saved is not suggested"
        );
        assert_eq!(
            dto.fields[3].suggestions,
            ["Go"],
            "neither is what is saved or avoided"
        );
    }

    #[test]
    fn the_card_serializes_in_camel_case() {
        let dto = ProfileDto::from_parts(&Preferences::default(), &PrefSuggestions::default(), false);
        let json = serde_json::to_value(&dto).unwrap_or_default();
        assert!(json.get("profilePath").is_none() && json.get("cv").is_none());
        assert_eq!(json["jobType"], "any");
        assert_eq!(json["canUndo"], false);
        assert_eq!(json["fields"][3]["key"], "skills");
        assert_eq!(json["fields"][0]["key"], "work_from");
        assert!(json["fields"][0]["hint"].is_string());
        assert!(
            json["fields"][1].get("hint").is_none(),
            "no hint, no key"
        );
        assert!(json["limits"]["maxNotesChars"]
            .as_u64()
            .is_some_and(|n| n > 0));
    }
}
