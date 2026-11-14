//! Small text helpers shared by the adapters: HTML fragments (HN comments, feed bodies) to plain
//! text, titles, and the length cap.

use std::sync::OnceLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use regex::Regex;

use crate::engine::pipeline::ingest::clean;

struct Patterns {
    blocks: Regex,
    dropped: Regex,
    tags: Regex,
    entity: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        blocks: Regex::new(r"(?i)<\s*(?:br|/p|p|/div|div|/li|/h[1-6]|h[1-6]|/tr|/pre|pre|/blockquote)\b[^>]*>").expect("static regex"),
        dropped: Regex::new(r"(?is)<\s*(script|style)\b.*?<\s*/\s*(?:script|style)\s*>").expect("static regex"),
        tags: Regex::new(r"(?s)<[^>]*>").expect("static regex"),
        entity: Regex::new(r"&(#[xX][0-9a-fA-F]{1,6}|#[0-9]{1,7}|[A-Za-z]{2,8});").expect("static regex"),
    })
}

/// The readable text of an HTML fragment: block tags become line breaks, other tags vanish,
/// entities are decoded.
pub fn html_to_text(html: &str) -> String {
    let p = patterns();
    let no_scripts = p.dropped.replace_all(html, " ");
    let broken = p.blocks.replace_all(&no_scripts, "\n");
    let bare = p.tags.replace_all(&broken, "");
    clean(&decode_entities(&bare))
}

/// `&amp;` `&#x27;` `&#8217;` ... Unknown names are left as written.
pub fn decode_entities(text: &str) -> String {
    patterns()
        .entity
        .replace_all(text, |caps: &regex::Captures| {
            let name = &caps[1];
            let decoded = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = name.strip_prefix('#') {
                dec.parse().ok().and_then(char::from_u32)
            } else {
                match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    "ndash" => Some('\u{2013}'),
                    "mdash" => Some('\u{2014}'),
                    "hellip" => Some('\u{2026}'),
                    "rsquo" | "lsquo" => Some('\''),
                    "rdquo" | "ldquo" => Some('"'),
                    _ => None,
                }
            };
            decoded.map_or_else(|| caps[0].to_string(), String::from)
        })
        .into_owned()
}

/// A date as feeds and search APIs write it: RFC 2822 (`Tue, 10 Nov 2026 12:00:00 GMT`), RFC 3339,
/// or a bare `2026-11-10` / `2026-11-10T12:00:00` / `2026-11-10 12:00:00` (taken as UTC).
pub fn parse_date(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(d) = DateTime::parse_from_rfc2822(text).or_else(|_| DateTime::parse_from_rfc3339(text)) {
        return Some(d.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(text, format) {
            return Some(d.and_utc());
        }
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|d| d.and_utc())
}

/// The first non-empty line of `text` without Markdown heading marks, at most `max` characters;
/// `None` when there is none.
pub fn first_line(text: &str, max: usize) -> Option<String> {
    let line = text.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty())?;
    Some(truncate(line, max))
}

/// At most `max` characters of `text`, with an ellipsis when something was cut.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('\u{2026}');
    cut
}

/// `text` cut to `max` characters (no ellipsis: the whole text is the item's, not a preview).
pub fn cap(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hn_comment_html_becomes_text() {
        let html = "Simple Analytics | DevOps Engineer | REMOTE | &#x24;5,000&#x2F;month<p>We use Rust &amp; Go. See <a href=\"https:&#x2F;&#x2F;x.test&#x2F;jobs\" rel=\"nofollow\">https:&#x2F;&#x2F;x.test&#x2F;jobs</a><p><pre><code>cargo run</code></pre><script>alert(1)</script>";
        let text = html_to_text(html);
        assert_eq!(
            text,
            "Simple Analytics | DevOps Engineer | REMOTE | $5,000/month\nWe use Rust & Go. See https://x.test/jobs\n\ncargo run"
        );
    }

    #[test]
    fn entities_and_unknown_names() {
        assert_eq!(decode_entities("a &lt;b&gt; &quot;c&quot; &#8217; &#x2019; &bogus; &"), "a <b> \"c\" \u{2019} \u{2019} &bogus; &");
        assert_eq!(decode_entities("&#99999999;"), "&#99999999;", "not a char");
    }

    #[test]
    fn dates_in_the_formats_feeds_and_apis_use() {
        let want = Some(1794312000);
        for text in [
            "Tue, 10 Nov 2026 12:00:00 GMT",
            "Tue, 10 Nov 2026 13:00:00 +0100",
            "2026-11-10T12:00:00Z",
            "2026-11-10T12:00:00+00:00",
            "2026-11-10T12:00:00",
            "2026-11-10 12:00:00",
            " 2026-11-10T12:00:00.000 ",
        ] {
            assert_eq!(parse_date(text).map(|d| d.timestamp()), want, "{text}");
        }
        assert_eq!(parse_date("2026-11-10").map(|d| d.timestamp()), Some(1794268800));
        assert_eq!(parse_date("yesterday"), None);
        assert_eq!(parse_date(""), None);
    }

    #[test]
    fn lines_and_limits() {
        assert_eq!(first_line("\n\n# Title here\n\nbody", 50).as_deref(), Some("Title here"));
        assert_eq!(first_line("  \n", 50), None);
        assert_eq!(first_line("abcdefghij", 5).as_deref(), Some("abcd\u{2026}"));
        assert_eq!(cap("héllo", 2), "hé");
        assert_eq!(cap("hi", 10), "hi");
    }
}
