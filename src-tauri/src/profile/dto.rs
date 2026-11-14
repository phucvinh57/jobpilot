//! Read models of this context (serde, camelCase). Core types stay free of serde and Tauri.

use std::path::Path;

use crate::engine::prefs::chat::{MAX_LIST_ITEMS, MAX_NOTES_CHARS};
use crate::engine::prefs::{PrefSuggestions, Preferences, UploadedCv};
use serde::Serialize;

use super::rules::{job_type_key, visible_suggestions, Field, MAX_ITEM_CHARS};

/// One tag field of the card: Work locations, Interested domains, Skills.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFieldDto {
    /// `locations` / `domains` / `skills`: what the web app sends back to add or remove an entry.
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub items: Vec<String>,
    /// Entries that could be added with one click (none of them is in the preferences already).
    pub suggestions: Vec<String>,
}

/// The CV attached for suggestions (context only: it is never copied into a CV).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CvDto {
    pub name: String,
    pub chars: usize,
    pub truncated: bool,
}

impl From<&UploadedCv> for CvDto {
    fn from(cv: &UploadedCv) -> Self {
        Self {
            name: cv.name.clone(),
            chars: cv.chars,
            truncated: cv.truncated,
        }
    }
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
    /// The master profile (`profile.yaml`) the CVs are written from.
    pub profile_path: String,
    /// `any` / `remote` / `hybrid` / `onsite`.
    pub job_type: &'static str,
    pub fields: Vec<ListFieldDto>,
    /// The user's own words, as saved.
    pub notes: String,
    pub cv: Option<CvDto>,
    pub can_undo: bool,
    pub limits: LimitsDto,
}

impl ProfileDto {
    /// The card around `prefs`.
    pub fn from_parts(
        profile_path: &Path,
        prefs: &Preferences,
        suggestions: &PrefSuggestions,
        cv: Option<&UploadedCv>,
        can_undo: bool,
    ) -> Self {
        let fields = Field::ALL
            .iter()
            .map(|&field| ListFieldDto {
                key: field.key(),
                label: field.label(),
                placeholder: field.placeholder(),
                items: field.list(prefs).clone(),
                suggestions: visible_suggestions(field, prefs, field.suggestions(suggestions)),
            })
            .collect();
        Self {
            profile_path: profile_path.display().to_string(),
            job_type: job_type_key(prefs.remote),
            fields,
            notes: prefs.notes.clone(),
            cv: cv.map(CvDto::from),
            can_undo,
            limits: LimitsDto {
                max_list_items: MAX_LIST_ITEMS,
                max_item_chars: MAX_ITEM_CHARS,
                max_notes_chars: MAX_NOTES_CHARS,
            },
        }
    }
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
    use std::path::Path;

    use crate::engine::prefs::{PrefSuggestions, Preferences, Remote, UploadedCv};

    use super::ProfileDto;

    #[test]
    fn the_card_lists_every_field_with_the_suggestions_that_can_still_be_added() {
        let prefs = Preferences {
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
        };
        let cv = UploadedCv {
            name: "cv.pdf".into(),
            chars: 1200,
            truncated: true,
        };
        let dto = ProfileDto::from_parts(
            Path::new("/home/me/profile.yaml"),
            &prefs,
            &suggestions,
            Some(&cv),
            true,
        );

        assert_eq!(dto.profile_path, "/home/me/profile.yaml");
        assert_eq!(dto.job_type, "hybrid");
        assert_eq!(dto.notes, "Senior only");
        assert!(dto.can_undo);
        assert_eq!(
            dto.cv
                .as_ref()
                .map(|c| (c.name.as_str(), c.chars, c.truncated)),
            Some(("cv.pdf", 1200, true))
        );

        let keys: Vec<_> = dto.fields.iter().map(|f| f.key).collect();
        assert_eq!(
            keys,
            ["locations", "domains", "skills"],
            "the order of the card"
        );
        assert_eq!(dto.fields[0].items, ["Hanoi"]);
        assert_eq!(
            dto.fields[0].suggestions,
            ["Remote"],
            "what is saved is not suggested"
        );
        assert_eq!(
            dto.fields[2].suggestions,
            ["Go"],
            "neither is what is saved or avoided"
        );
    }

    #[test]
    fn the_card_serializes_in_camel_case() {
        let dto = ProfileDto::from_parts(
            Path::new("p.yaml"),
            &Preferences::default(),
            &PrefSuggestions::default(),
            None,
            false,
        );
        let json = serde_json::to_value(&dto).unwrap_or_default();
        assert_eq!(json["profilePath"], "p.yaml");
        assert_eq!(json["jobType"], "any");
        assert_eq!(json["canUndo"], false);
        assert!(json["cv"].is_null());
        assert_eq!(json["fields"][2]["key"], "skills");
        assert!(json["limits"]["maxNotesChars"]
            .as_u64()
            .is_some_and(|n| n > 0));
    }
}
