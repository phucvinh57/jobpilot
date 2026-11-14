//! Rules of the preference lists that two contexts apply (the Profile form and the Trends
//! "Add to preferences" chip): how an entry is cleaned and compared, when a list takes one more,
//! and where a topic stands in the saved preferences.

use jobpilot_core::prefs::chat::MAX_LIST_ITEMS;

/// Longest list entry, in characters.
pub const MAX_ITEM_CHARS: usize = 80;

/// Whether a topic is in the preferences already, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    /// In none of the lists.
    Absent,
    /// In a list of wanted things (must or nice skills, domains).
    Added,
    /// In a list of things to avoid.
    Avoided,
}

impl Presence {
    pub fn as_str(self) -> &'static str {
        match self {
            Presence::Absent => "absent",
            Presence::Added => "added",
            Presence::Avoided => "avoided",
        }
    }
}

/// A list entry as it is stored: trimmed and cut to [`MAX_ITEM_CHARS`].
pub fn item_text(text: &str) -> String {
    text.trim().chars().take(MAX_ITEM_CHARS).collect()
}

/// What two entries are compared by: [`item_text`] in Unicode lowercase (the same as
/// `Preferences::normalized`, so what the screen calls a duplicate is one there too).
pub fn item_key(text: &str) -> String {
    item_text(text).to_lowercase()
}

/// `list` plus `text`; `None` when there is nothing to add (blank, already there ignoring case, full).
pub fn with_item(list: &[String], text: &str) -> Option<Vec<String>> {
    let text = item_text(text);
    let key = text.to_lowercase();
    if text.is_empty()
        || list.len() >= MAX_LIST_ITEMS
        || list.iter().any(|i| i.trim().to_lowercase() == key)
    {
        return None;
    }
    let mut out = list.to_vec();
    out.push(text);
    Some(out)
}

/// Where `text` stands among the `wanted` lists and the `avoided` ones (ignoring case).
pub fn presence_in(wanted: &[&[String]], avoided: &[&[String]], text: &str) -> Presence {
    let key = item_key(text);
    if key.is_empty() {
        return Presence::Absent;
    }
    let has = |list: &[String]| list.iter().any(|i| i.trim().to_lowercase() == key);
    if wanted.iter().any(|l| has(l)) {
        Presence::Added
    } else if avoided.iter().any(|l| has(l)) {
        Presence::Avoided
    } else {
        Presence::Absent
    }
}

/// Why `text` is not in a list of `list_len` entries after an attempt to add it: it is in the
/// avoid list, the list really is full, or something else went wrong.
pub fn why_not_added(presence: Presence, list_len: usize, text: &str) -> String {
    let text = item_text(text);
    if presence == Presence::Avoided {
        format!("\"{text}\" is in your avoid list, so it was not added.")
    } else if list_len >= MAX_LIST_ITEMS {
        format!(
            "Could not add \"{text}\": a list of preferences holds up to {MAX_LIST_ITEMS} entries."
        )
    } else {
        format!("Could not add \"{text}\".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_topic_is_found_in_the_wanted_lists_before_the_avoided_ones() {
        let wanted = vec!["Rust".to_string()];
        let avoided = vec!["PHP".to_string()];
        assert_eq!(
            presence_in(&[&wanted], &[&avoided], " rust "),
            Presence::Added
        );
        assert_eq!(
            presence_in(&[&wanted], &[&avoided], "php"),
            Presence::Avoided
        );
        assert_eq!(presence_in(&[&wanted], &[&avoided], "Go"), Presence::Absent);
        assert_eq!(presence_in(&[&wanted], &[&avoided], "  "), Presence::Absent);
    }

    #[test]
    fn the_reason_names_the_avoid_list_or_the_full_list() {
        assert!(why_not_added(Presence::Avoided, 0, "PHP").contains("avoid list"));
        assert!(why_not_added(Presence::Absent, MAX_LIST_ITEMS, "Go").contains("holds up to"));
        assert_eq!(
            why_not_added(Presence::Absent, 0, " Go "),
            "Could not add \"Go\"."
        );
    }
}
