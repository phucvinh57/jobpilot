//! Pure rules of this context, unit-tested here: how a list entry is cleaned and compared, when a
//! topic may be added, what the line under the notes says. Ported from the GPUI card.
//!
//! The rules that only the screen needs (whether Save is on, when a "Saved" label shows and how it
//! fades) live in the web app's `contexts/profile/domain`.

use std::path::Path;

use crate::shared::topics::{self, presence_in};
pub use crate::shared::topics::{item_text, with_item, Presence, MAX_ITEM_CHARS};
use crate::engine::health::{classify, Problem};
use crate::engine::prefs::chat::MAX_LIST_ITEMS;

use crate::engine::prefs::{NotesRead, PrefSuggestions, Preferences, Remote};

/// Suggestion chips shown under a field.
pub const SUGGESTION_CHIPS: usize = 8;

/// A list of the preferences that is edited as a tag field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Locations,
    Domains,
    Skills,
}

impl Field {
    pub const ALL: [Field; 3] = [Field::Locations, Field::Domains, Field::Skills];

    /// Used in element ids and as the name the web app sends.
    pub fn key(self) -> &'static str {
        match self {
            Field::Locations => "locations",
            Field::Domains => "domains",
            Field::Skills => "skills",
        }
    }

    /// The field named `key` (see [`key`](Self::key)).
    pub fn parse(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Field::Locations => "Work locations",
            Field::Domains => "Interested domains",
            Field::Skills => "Skills",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Field::Locations => "Add a city or country",
            Field::Domains => "Add a domain, e.g. fintech",
            Field::Skills => "Add a skill",
        }
    }

    pub fn list(self, prefs: &Preferences) -> &Vec<String> {
        match self {
            Field::Locations => &prefs.locations,
            Field::Domains => &prefs.domains,
            Field::Skills => &prefs.must_skills,
        }
    }

    pub fn list_mut(self, prefs: &mut Preferences) -> &mut Vec<String> {
        match self {
            Field::Locations => &mut prefs.locations,
            Field::Domains => &mut prefs.domains,
            Field::Skills => &mut prefs.must_skills,
        }
    }

    pub fn suggestions(self, all: &PrefSuggestions) -> &Vec<String> {
        match self {
            Field::Locations => &all.locations,
            Field::Domains => &all.domains,
            Field::Skills => &all.skills,
        }
    }

    /// Add `text` to this list of `prefs`; `false` when there was nothing to add (see [`with_item`])
    /// or it is in one of the related lists already (see [`presence`](Self::presence)).
    pub fn add(self, prefs: &mut Preferences, text: &str) -> bool {
        if self.presence(prefs, text) != Presence::Absent {
            return false;
        }
        match with_item(self.list(prefs), text) {
            Some(list) => {
                *self.list_mut(prefs) = list;
                true
            }
            None => false,
        }
    }

    /// Where `text` stands in `prefs` (ignoring case, with Unicode-aware lowercase like
    /// `Preferences::normalized`): skills count in the must, nice and avoid lists, domains in the
    /// interested and avoided ones.
    pub fn presence(self, prefs: &Preferences, text: &str) -> Presence {
        let (wanted, avoided): (&[&[String]], &[&[String]]) = match self {
            Field::Locations => (&[&prefs.locations], &[]),
            Field::Domains => (&[&prefs.domains], &[&prefs.avoid_domains]),
            Field::Skills => (
                &[&prefs.must_skills, &prefs.nice_skills],
                &[&prefs.avoid_skills],
            ),
        };
        presence_in(wanted, avoided, text)
    }

    /// Why `text` is not in `prefs` after an attempt to add it: it is in the avoid list, the list
    /// really is full, or something else went wrong.
    pub fn why_not_added(self, prefs: &Preferences, text: &str) -> String {
        topics::why_not_added(self.presence(prefs, text), self.list(prefs).len(), text)
    }

    /// `text` could be added now: in none of the related lists, and this one is not full.
    pub fn can_add(self, prefs: &Preferences, text: &str) -> bool {
        self.presence(prefs, text) == Presence::Absent
            && with_item(self.list(prefs), text).is_some()
    }
}

/// What to tell the user who typed `text` into a field when it is in a related list already
/// (`None` when it is not).
pub fn already_there_message(presence: Presence, text: &str) -> Option<String> {
    let text = item_text(text);
    match presence {
        Presence::Absent => None,
        Presence::Added => Some(format!("\"{text}\" is already in your preferences.")),
        Presence::Avoided => Some(format!(
            "\"{text}\" is in your avoid list, so it was not added."
        )),
    }
}

/// What an attempt to add a topic comes to, decided on the document as it is when the edit runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddCheck {
    /// Add it.
    Add,
    /// Nothing to add (blank, or in this very list already): no message.
    Ignore,
    /// Not added, and the user is told why.
    Refuse(String),
}

/// The checks before an add, in the order the card made them: a full list, a no-op, a related list.
pub fn add_check(field: Field, prefs: &Preferences, text: &str) -> AddCheck {
    let list = field.list(prefs);
    if list.len() >= MAX_LIST_ITEMS {
        return AddCheck::Refuse(format!(
            "{} can hold up to {MAX_LIST_ITEMS} entries.",
            field.label()
        ));
    }
    if with_item(list, text).is_none() {
        return AddCheck::Ignore;
    }
    // In a list the form does not show (nice or avoided skills, avoided domains): say so.
    match already_there_message(field.presence(prefs, text), text) {
        Some(message) => AddCheck::Refuse(message),
        None => AddCheck::Add,
    }
}

/// `list` without the entries equal to `item` (ignoring case): by value, so a stale click or a
/// list that changed meanwhile never removes another entry.
pub fn without_item(list: &[String], item: &str) -> Vec<String> {
    let key = item.trim().to_lowercase();
    list.iter()
        .filter(|i| i.trim().to_lowercase() != key)
        .cloned()
        .collect()
}

/// The suggestions to show: the ones that could be added to `field` (not in it or its related
/// lists yet), at most [`SUGGESTION_CHIPS`].
pub fn visible_suggestions(field: Field, prefs: &Preferences, all: &[String]) -> Vec<String> {
    all.iter()
        .filter(|s| field.can_add(prefs, s))
        .take(SUGGESTION_CHIPS)
        .cloned()
        .collect()
}

// ------------------------------------------------------------------ job type

/// The name of a job type as the web app sends and reads it.
pub fn job_type_key(remote: Remote) -> &'static str {
    match remote {
        Remote::Any => "any",
        Remote::Remote => "remote",
        Remote::Hybrid => "hybrid",
        Remote::Onsite => "onsite",
    }
}

/// The job type named `key` (see [`job_type_key`]).
pub fn parse_job_type(key: &str) -> Option<Remote> {
    [Remote::Any, Remote::Remote, Remote::Hybrid, Remote::Onsite]
        .into_iter()
        .find(|r| job_type_key(*r) == key)
}

// ------------------------------------------------------------------ notes

/// What the line under the notes says once the model is done with them.
#[derive(Debug, PartialEq, Eq)]
pub enum NotesStatus {
    /// Nothing to say.
    Idle,
    /// A muted line, never a blocker.
    Failed(String),
}

/// Said under the notes when the model could not read all of them (it asked a question instead).
pub const PARTLY_READ: &str = "Your notes are saved; I could not read everything from them.";

/// What to tell the user when the notes were saved but could not be read by the model.
pub fn unread_notes_message(error: &str) -> String {
    match classify(error) {
        Problem::NoProvider => {
            "Your notes are saved. Set up an AI provider below and the app can also read them for roles, seniority and salary."
                .to_string()
        }
        Problem::Network => "Your notes are saved, but the AI could not be reached to read them. Save them again to retry.".to_string(),
        _ => {
            let short: String = error.chars().take(160).collect();
            format!("Your notes are saved. They could not be read for more details: {short}")
        }
    }
}

/// What the line under the notes says once the model is done with them.
pub fn notes_status(read: NotesRead) -> NotesStatus {
    match read {
        NotesRead::Read => NotesStatus::Idle,
        NotesRead::PartlyRead => NotesStatus::Failed(PARTLY_READ.to_string()),
        // The failure may be one the user can act on (no provider, no network).
        NotesRead::Failed(message) => NotesStatus::Failed(unread_notes_message(&message)),
        // The user undid a change meanwhile: the read was dropped and there is nothing to say.
        NotesRead::Stale => NotesStatus::Idle,
    }
}

// ------------------------------------------------------------------ profile file

/// A profile is a YAML file.
pub fn is_profile_file_name(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    ext == "yaml" || ext == "yml"
}

#[cfg(test)]
mod tests {
    use crate::shared::topics::item_key;
    use std::path::Path;

    use crate::engine::prefs::chat::MAX_LIST_ITEMS;
    use crate::engine::prefs::{NotesRead, PrefSuggestions, Preferences, Remote};

    use super::{
        add_check, already_there_message, is_profile_file_name, job_type_key, notes_status,
        parse_job_type, unread_notes_message, visible_suggestions, with_item, without_item,
        AddCheck, Field, NotesStatus, Presence, MAX_ITEM_CHARS, PARTLY_READ, SUGGESTION_CHIPS,
    };

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn items_are_added_without_blanks_duplicates_or_overflow() {
        let list = strings(&["Rust"]);
        assert_eq!(with_item(&list, "  Go "), Some(strings(&["Rust", "Go"])));
        assert_eq!(
            with_item(&list, "rust"),
            None,
            "already there, ignoring case"
        );
        assert_eq!(with_item(&list, "   "), None);
        let full: Vec<String> = (0..MAX_LIST_ITEMS).map(|i| format!("s{i}")).collect();
        assert_eq!(with_item(&full, "one more"), None);
        assert_eq!(
            with_item(&[], &"x".repeat(500)).unwrap_or_default()[0]
                .chars()
                .count(),
            MAX_ITEM_CHARS
        );
    }

    #[test]
    fn items_are_compared_with_unicode_lowercase() {
        let list = strings(&["Z\u{fc}rich", "\u{c9}cole"]);
        assert_eq!(
            with_item(&list, "Z\u{dc}RICH"),
            None,
            "the same city in capitals"
        );
        assert_eq!(with_item(&list, " \u{e9}cole "), None);
        assert!(
            with_item(&list, "Zurich").is_some(),
            "another spelling is another entry"
        );
        assert_eq!(item_key("  \u{c9}COLE "), "\u{e9}cole");
    }

    #[test]
    fn an_item_is_removed_by_value_not_by_position() {
        assert_eq!(without_item(&strings(&["a", "b"]), "a"), strings(&["b"]));
        assert_eq!(
            without_item(&strings(&["Rust", "Go"]), " rust "),
            strings(&["Go"]),
            "ignoring case and spaces"
        );
        assert_eq!(
            without_item(&strings(&["a"]), "zzz"),
            strings(&["a"]),
            "an item that is gone changes nothing"
        );
        // A stale click: the list lost its first entry since the chip for "b" was drawn.
        let now = strings(&["b", "c"]);
        assert_eq!(
            without_item(&now, "c"),
            strings(&["b"]),
            "removes the entry that was clicked, whatever its place"
        );
        assert_eq!(without_item(&[], "x"), Vec::<String>::new());
    }

    #[test]
    fn a_topic_counts_as_present_in_every_related_list() {
        let prefs = Preferences {
            must_skills: strings(&["Rust"]),
            nice_skills: strings(&["Kafka"]),
            avoid_skills: strings(&["PHP"]),
            domains: strings(&["Fintech"]),
            avoid_domains: strings(&["Gambling"]),
            ..Default::default()
        };
        let skills = |t: &str| Field::Skills.presence(&prefs, t);
        assert_eq!(skills("rust"), Presence::Added);
        assert_eq!(
            skills(" KAFKA "),
            Presence::Added,
            "a nice skill is present too"
        );
        assert_eq!(skills("php"), Presence::Avoided);
        assert_eq!(skills("Go"), Presence::Absent);
        assert_eq!(skills("  "), Presence::Absent);
        let domains = |t: &str| Field::Domains.presence(&prefs, t);
        assert_eq!(domains("FINTECH"), Presence::Added);
        assert_eq!(domains("gambling"), Presence::Avoided);
        assert_eq!(
            domains("rust"),
            Presence::Absent,
            "skills and domains are separate"
        );
        assert_eq!(Field::Locations.presence(&prefs, "Rust"), Presence::Absent);

        // Unicode-aware, like `Preferences::normalized`.
        let accented = Preferences {
            domains: strings(&["\u{c9}nergie"]),
            ..Default::default()
        };
        assert_eq!(
            Field::Domains.presence(&accented, "\u{e9}NERGIE"),
            Presence::Added
        );

        // Adding never duplicates into another list, and never adds what is avoided.
        let mut prefs = prefs;
        assert!(!Field::Skills.add(&mut prefs, "kafka"));
        assert!(!Field::Skills.add(&mut prefs, "PHP"));
        assert!(!Field::Domains.add(&mut prefs, "Gambling"));
        assert!(Field::Skills.add(&mut prefs, "Go"));
        assert_eq!(prefs.must_skills, strings(&["Rust", "Go"]));
        assert_eq!(prefs.nice_skills, strings(&["Kafka"]));
    }

    #[test]
    fn the_reason_a_topic_was_not_added_is_the_real_one() {
        let mut prefs = Preferences {
            avoid_skills: strings(&["PHP"]),
            ..Default::default()
        };
        assert!(Field::Skills
            .why_not_added(&prefs, "php")
            .contains("avoid list"));
        let other = Field::Skills.why_not_added(&prefs, "Go");
        assert_eq!(
            other, "Could not add \"Go\".",
            "not full: no talk of a full list"
        );
        assert!(!other.contains("holds up to"));
        prefs.must_skills = (0..MAX_LIST_ITEMS).map(|i| format!("s{i}")).collect();
        assert!(Field::Skills
            .why_not_added(&prefs, "Go")
            .contains(&format!("holds up to {MAX_LIST_ITEMS}")));
        assert!(
            !Field::Domains
                .why_not_added(&prefs, "Go")
                .contains("holds up to"),
            "only the list that is full"
        );
        assert_eq!(already_there_message(Presence::Absent, "x"), None);
        assert!(already_there_message(Presence::Added, "x").is_some_and(|m| m.contains("already")));
        assert!(
            already_there_message(Presence::Avoided, "x").is_some_and(|m| m.contains("avoid list"))
        );
    }

    #[test]
    fn every_field_reads_and_writes_its_own_list() {
        let mut prefs = Preferences::default();
        for field in Field::ALL {
            assert!(field.add(&mut prefs, &format!("item {}", field.key())));
        }
        assert_eq!(prefs.locations, strings(&["item locations"]));
        assert_eq!(prefs.domains, strings(&["item domains"]));
        assert_eq!(
            prefs.must_skills,
            strings(&["item skills"]),
            "skills are the must-have skills"
        );
        assert!(
            prefs.nice_skills.is_empty() && prefs.avoid_skills.is_empty(),
            "the others are left alone"
        );
        for field in Field::ALL {
            assert_eq!(
                field.presence(&prefs, &format!(" ITEM {} ", field.key())),
                Presence::Added
            );
            assert!(
                !field.add(&mut prefs, &format!("item {}", field.key())),
                "no duplicate"
            );
        }
        let keys: std::collections::HashSet<_> = Field::ALL.iter().map(|f| f.key()).collect();
        assert_eq!(keys.len(), Field::ALL.len(), "element ids stay unique");
        // The web app names a field by its key.
        for field in Field::ALL {
            assert_eq!(Field::parse(field.key()), Some(field));
        }
        assert_eq!(Field::parse("roles"), None);
    }

    #[test]
    fn suggestions_skip_what_is_saved_and_are_capped() {
        let all: Vec<String> = (0..20).map(|i| format!("s{i}")).collect();
        let mut prefs = Preferences {
            must_skills: strings(&["S0", "s3"]),
            ..Default::default()
        };
        let shown = visible_suggestions(Field::Skills, &prefs, &all);
        assert_eq!(shown.len(), SUGGESTION_CHIPS);
        assert!(!shown.contains(&"s0".to_string()) && !shown.contains(&"s3".to_string()));
        assert_eq!(shown[0], "s1");
        // A nice or avoided skill is not suggested again either.
        prefs.nice_skills = strings(&["s1"]);
        prefs.avoid_skills = strings(&["S2"]);
        assert_eq!(visible_suggestions(Field::Skills, &prefs, &all)[0], "s4");
        let full = Preferences {
            must_skills: all.clone(),
            ..Default::default()
        };
        assert!(visible_suggestions(Field::Skills, &full, &all).is_empty());
        let none = PrefSuggestions::default();
        assert!(Field::Skills.suggestions(&none).is_empty());
    }

    #[test]
    fn what_the_model_did_with_the_notes_becomes_the_line_under_them() {
        assert_eq!(
            notes_status(NotesRead::Read),
            NotesStatus::Idle,
            "a clean read says nothing"
        );
        assert_eq!(
            notes_status(NotesRead::PartlyRead),
            NotesStatus::Failed(PARTLY_READ.to_string()),
            "a question nobody can answer here is a muted line"
        );
        match notes_status(NotesRead::Failed("no AI provider is set up".into())) {
            NotesStatus::Failed(line) => assert!(
                line.contains("AI provider"),
                "an actionable failure keeps its advice: {line}"
            ),
            NotesStatus::Idle => panic!("a failure is a muted line"),
        }
        match notes_status(NotesRead::Failed("odd".into())) {
            NotesStatus::Failed(line) => {
                assert!(line.starts_with("Your notes are saved"), "{line}")
            }
            NotesStatus::Idle => panic!("a failure is a muted line"),
        }
        assert_eq!(
            notes_status(NotesRead::Stale),
            NotesStatus::Idle,
            "a read dropped by an undo says nothing"
        );
        assert_eq!(
            PARTLY_READ,
            "Your notes are saved; I could not read everything from them."
        );
    }

    #[test]
    fn an_unread_note_says_what_to_do() {
        assert!(unread_notes_message("something odd").starts_with("Your notes are saved."));
        let long = unread_notes_message(&"e".repeat(500));
        assert!(long.chars().count() < 300, "the error is cut");
        assert!(
            unread_notes_message("connection refused").contains("Save them again"),
            "a network failure is retried by saving again"
        );
    }

    #[test]
    fn an_add_is_checked_on_the_document_as_it_is() {
        let prefs = Preferences {
            must_skills: strings(&["Rust"]),
            nice_skills: strings(&["Kafka"]),
            avoid_skills: strings(&["PHP"]),
            ..Default::default()
        };
        assert_eq!(add_check(Field::Skills, &prefs, "Go"), AddCheck::Add);
        assert_eq!(
            add_check(Field::Skills, &prefs, "  "),
            AddCheck::Ignore,
            "blank: nothing to say"
        );
        assert_eq!(
            add_check(Field::Skills, &prefs, "rust"),
            AddCheck::Ignore,
            "in this very list: nothing to say"
        );
        assert!(
            matches!(add_check(Field::Skills, &prefs, "kafka"), AddCheck::Refuse(m) if m.contains("already"))
        );
        assert!(
            matches!(add_check(Field::Skills, &prefs, "php"), AddCheck::Refuse(m) if m.contains("avoid list"))
        );

        // A full list says so, before anything else.
        let full = Preferences {
            must_skills: (0..MAX_LIST_ITEMS).map(|i| format!("s{i}")).collect(),
            ..Default::default()
        };
        let refused = add_check(Field::Skills, &full, "Go");
        assert_eq!(
            refused,
            AddCheck::Refuse(format!("Skills can hold up to {MAX_LIST_ITEMS} entries."))
        );
        assert_eq!(
            add_check(Field::Domains, &full, "Go"),
            AddCheck::Add,
            "only the list that is full"
        );
    }

    #[test]
    fn job_types_are_named_and_parsed_both_ways() {
        for remote in [Remote::Any, Remote::Remote, Remote::Hybrid, Remote::Onsite] {
            assert_eq!(parse_job_type(job_type_key(remote)), Some(remote));
        }
        assert_eq!(job_type_key(Remote::Onsite), "onsite");
        assert_eq!(parse_job_type("moon"), None);
    }

    #[test]
    fn a_profile_is_a_yaml_file() {
        assert!(is_profile_file_name(Path::new("/home/me/profile.yaml")));
        assert!(is_profile_file_name(Path::new("profile.YML")));
        assert!(!is_profile_file_name(Path::new("profile.json")));
        assert!(!is_profile_file_name(Path::new("profile")));
    }
}
