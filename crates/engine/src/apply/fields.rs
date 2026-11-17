//! Types shared by extraction, answering, review and filling.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Email,
    Tel,
    Url,
    Number,
    Date,
    Textarea,
    Select,
    Radio,
    Checkbox,
    CheckboxGroup,
    File,
    /// Type-ahead dropdown (`role=combobox`); the options only appear while typing.
    Combobox,
    #[default]
    #[serde(other)]
    Text,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Email => "email",
            FieldKind::Tel => "tel",
            FieldKind::Url => "url",
            FieldKind::Number => "number",
            FieldKind::Date => "date",
            FieldKind::Textarea => "textarea",
            FieldKind::Select => "select",
            FieldKind::Radio => "radio",
            FieldKind::Checkbox => "checkbox",
            FieldKind::CheckboxGroup => "checkbox_group",
            FieldKind::File => "file",
            FieldKind::Combobox => "combobox",
        }
    }

    /// The value must be one of [`FormField::options`].
    pub fn is_choice(self) -> bool {
        matches!(self, FieldKind::Select | FieldKind::Radio | FieldKind::CheckboxGroup)
    }
}

/// One fillable field of the form, as read from the page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FormField {
    /// `jp<step>_<n>`, the `data-jp-id` attribute set on the element(s).
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
    pub required: bool,
    /// Option labels for select / radio / checkbox group.
    pub options: Vec<String>,
    pub current_value: String,
    pub name: String,
    pub maxlength: Option<usize>,
    /// `accept` of a file input.
    pub accept: String,
    pub step: usize,
}

/// Where an answer came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Profile basics or a remembered answer.
    Profile,
    /// The tailored CV.
    Cv,
    /// Written by the model from profile + CV + JD.
    Generated,
    /// Nobody knows: the user has to answer.
    Unknown,
    /// Already filled in by the site (e.g. LinkedIn's saved contact info).
    Page,
    /// Typed by the user in the review.
    User,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Profile => "profile",
            Source::Cv => "cv",
            Source::Generated => "generated",
            Source::Unknown => "unknown",
            Source::Page => "page",
            Source::User => "you",
        }
    }
}

/// The value chosen for one field.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub id: String,
    /// Empty = leave the field alone.
    pub value: String,
    pub source: Source,
    pub needs_input: bool,
    /// Something to double check ("number 7 is not in your profile", "value rejected: ...").
    pub warning: Option<String>,
}

impl Answer {
    pub fn unknown(id: &str, warning: Option<String>) -> Self {
        Answer { id: id.to_string(), value: String::new(), source: Source::Unknown, needs_input: true, warning }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_parse_from_the_page_json() {
        let json = serde_json::json!([
            {"id": "jp0_0", "label": "Email", "type": "email", "required": true, "options": [], "current_value": "", "name": "email", "step": 0},
            {"id": "jp0_1", "label": "Plan", "type": "checkbox_group", "options": ["a", "b"]},
            {"id": "jp0_2", "label": "Odd", "type": "datetime-local"},
            {"id": "jp0_3", "label": "Resume", "type": "file", "accept": ".pdf", "maxlength": 5}
        ]);
        let fields: Vec<FormField> = serde_json::from_value(json).unwrap();
        assert_eq!(fields[0].kind, FieldKind::Email);
        assert!(fields[0].required);
        assert_eq!(fields[1].kind, FieldKind::CheckboxGroup);
        assert!(fields[1].kind.is_choice());
        assert_eq!(fields[2].kind, FieldKind::Text, "unknown types fall back to text");
        assert_eq!(fields[3].maxlength, Some(5));
    }
}
