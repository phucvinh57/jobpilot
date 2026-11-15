//! Plain text -> Typst markup.
//!
//! `assets/cv.typ` evaluates some fields as markup (`md`: summary lines, bullets, group titles,
//! project intros), so a `#`, `$`, `@` or `*` the user wrote would run as code or change the
//! format. The reader asks the model for plain text and escapes it here, so a CV renders exactly
//! the user's words.

use crate::engine::pipeline::cv::Cv;

/// Characters with a meaning in Typst markup, escaped anywhere.
const SPECIAL: &[char] = &['\\', '#', '$', '@', '*', '_', '<', '>', '[', ']', '`', '~'];
/// Characters that start a heading, list, enum or term only at the start of a line.
const LINE_START: &[char] = &['=', '-', '+', '/'];

/// `text` as Typst markup that shows exactly `text`.
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        let at_start = out.trim().is_empty();
        let needs_escape = SPECIAL.contains(&c)
            // `//` and `/*` start comments
            || (c == '/' && matches!(next, Some('/' | '*')))
            || (at_start && LINE_START.contains(&c))
            // `1. ` at the start is a numbered list item
            || (c == '.' && i > 0 && chars[..i].iter().all(char::is_ascii_digit) && next.is_some_and(char::is_whitespace));
        if needs_escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape the fields `cv.typ` evaluates as markup.
pub fn escape_cv(cv: &mut Cv) {
    let escape_all = |items: &mut Vec<String>| items.iter_mut().for_each(|s| *s = escape_markup(s));
    escape_all(&mut cv.summary);
    for e in &mut cv.experience {
        escape_all(&mut e.bullets);
        for g in &mut e.groups {
            g.title = escape_markup(&g.title);
            escape_all(&mut g.bullets);
        }
    }
    for p in &mut cv.projects {
        p.intro = escape_markup(&p.intro);
        escape_all(&mut p.bullets);
    }
    for e in &mut cv.education {
        escape_all(&mut e.bullets);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_characters_are_escaped_and_plain_text_is_kept() {
        assert_eq!(escape_markup("Built a REST API in Rust."), "Built a REST API in Rust.");
        assert_eq!(escape_markup("C# and $5 at @acme"), r"C\# and \$5 at \@acme");
        assert_eq!(escape_markup("*bold* _it_ <tag> [x] `code` ~"), r"\*bold\* \_it\_ \<tag\> \[x\] \`code\` \~");
        assert_eq!(escape_markup(r"a\b"), r"a\\b");
        assert_eq!(escape_markup("x // y /* z"), r"x \// y \/\* z");
        assert_eq!(escape_markup("see https://a.dev/x"), r"see https:\//a.dev/x");
    }

    #[test]
    fn line_start_markers_are_escaped_only_at_the_start() {
        assert_eq!(escape_markup("- item"), r"\- item");
        assert_eq!(escape_markup("= Title"), r"\= Title");
        assert_eq!(escape_markup("+ more"), r"\+ more");
        assert_eq!(escape_markup("2020. Founded"), r"2020\. Founded");
        assert_eq!(escape_markup("a - b = c + d"), "a - b = c + d");
        assert_eq!(escape_markup("Version 2.0 shipped"), "Version 2.0 shipped");
        assert_eq!(escape_markup("3.5 years"), "3.5 years");
    }
}
