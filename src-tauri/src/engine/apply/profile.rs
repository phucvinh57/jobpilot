//! The `application:` section of `profile.yaml`: reusable answers to questions ATS forms ask
//! (work authorization, notice period, salary expectation, ...) as a `question/key -> answer` map.
//!
//! Reading uses serde. Writing never re-serializes the whole file: only the `application:` top-level
//! block is replaced (or appended when it does not exist yet), so comments, ordering and formatting
//! of the rest of the profile stay byte-for-byte as the user wrote them.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_yaml::{Mapping, Value};

/// Top-level key of the section.
pub const KEY: &str = "application";

/// Comment written above a block this module appends.
const HEADER: &str = "# Reusable application answers (question -> answer). JobPilot adds to this section; edit freely.";

/// Lower-case alphanumeric words of a question, for matching keys and labels.
pub fn normalize_key(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

/// The answers of a profile document (`[]` when there is no `application:` section).
pub fn parse_answers(yaml: &str) -> Result<Vec<(String, String)>> {
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc: Value = serde_yaml::from_str(yaml).context("profile.yaml is not valid YAML")?;
    let Some(section) = doc.get(KEY) else { return Ok(Vec::new()) };
    match section {
        Value::Null => Ok(Vec::new()),
        Value::Mapping(map) => Ok(map.iter().filter_map(|(k, v)| Some((scalar(k)?, scalar(v)?))).collect()),
        _ => bail!("`{KEY}:` in the profile must be a mapping of question -> answer"),
    }
}

/// Answers saved in the profile file at `path` (`[]` when the file does not exist).
pub fn read_answers(path: &Path) -> Result<Vec<(String, String)>> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_answers(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Save `new` answers into the profile at `path`: existing questions (compared loosely) get the new
/// answer, the others are appended. Only the `application:` block is touched. Returns how many
/// answers were added or changed. The file is replaced atomically.
pub fn remember(path: &Path, new: &[(String, String)]) -> Result<usize> {
    let new: Vec<(String, String)> =
        new.iter().map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).filter(|(k, v)| !k.is_empty() && !v.is_empty()).collect();
    if new.is_empty() {
        return Ok(0);
    }
    let path = resolve(path);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let (updated, changed) = merge(&text, &new)?;
    if changed > 0 {
        write_atomic(&path, &updated)?;
    }
    Ok(changed)
}

/// Pure version of [`remember`]: the new document text and how many answers changed.
pub fn merge(text: &str, new: &[(String, String)]) -> Result<(String, usize)> {
    let mut answers = parse_answers(text)?;
    let mut changed = 0;
    for (key, value) in new {
        let wanted = normalize_key(key);
        match answers.iter_mut().find(|(k, _)| normalize_key(k) == wanted) {
            Some((_, existing)) if existing == value => {}
            Some((_, existing)) => {
                *existing = value.clone();
                changed += 1;
            }
            None => {
                answers.push((key.clone(), value.clone()));
                changed += 1;
            }
        }
    }
    if changed == 0 {
        return Ok((text.to_string(), 0));
    }
    let updated = rewrite_block(text, &answers)?;
    // Whatever we wrote must still be a document with the same answers.
    let check = parse_answers(&updated).context("the rewritten profile does not parse; nothing was saved")?;
    if check != answers {
        bail!("the rewritten `{KEY}:` block does not read back identically; nothing was saved");
    }
    Ok((updated, changed))
}

/// Replace the `application:` block of `text` with `answers` (appending it when missing).
pub fn rewrite_block(text: &str, answers: &[(String, String)]) -> Result<String> {
    let block = render_block(answers)?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    match find_block(&lines) {
        Some((start, end)) => {
            let mut out: String = lines[..start].concat();
            out.push_str(&block);
            // Keep the separation that followed the old block.
            let rest = lines[end..].concat();
            out.push_str(&rest);
            Ok(out)
        }
        None => {
            let mut out = text.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(HEADER);
            out.push('\n');
            out.push_str(&block);
            Ok(out)
        }
    }
}

fn render_block(answers: &[(String, String)]) -> Result<String> {
    let mut map = Mapping::new();
    for (k, v) in answers {
        map.insert(Value::String(k.clone()), Value::String(v.clone()));
    }
    let mut root = Mapping::new();
    root.insert(Value::String(KEY.into()), Value::Mapping(map));
    let mut text = serde_yaml::to_string(&Value::Mapping(root)).context("serializing the answers")?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// `[start, end)` line range of the top-level `application:` block, without the blank lines and
/// column-0 comments that precede the next top-level key (those belong to it).
fn find_block(lines: &[&str]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| top_level_key(l).is_some_and(|k| k == KEY))?;
    let mut end = start + 1;
    while end < lines.len() && !is_top_level_content(lines[end]) {
        end += 1;
    }
    while end > start + 1 && is_blank_or_comment0(lines[end - 1]) {
        end -= 1;
    }
    Some((start, end))
}

/// A column-0 line that is not a comment or blank: it starts the next top-level node.
fn is_top_level_content(line: &str) -> bool {
    line.chars().next().is_some_and(|c| !c.is_whitespace() && c != '#')
}

fn is_blank_or_comment0(line: &str) -> bool {
    line.trim().is_empty() || line.starts_with('#')
}

/// The key of a column-0 `key:` line (plain or quoted).
fn top_level_key(line: &str) -> Option<String> {
    if !is_top_level_content(line) || line.starts_with("- ") || line.starts_with("---") || line.starts_with("...") {
        return None;
    }
    let line = line.trim_end();
    let (key, rest) = match line.chars().next()? {
        q @ ('"' | '\'') => {
            let close = line[1..].find(q)? + 1;
            (line[1..close].to_string(), &line[close + 1..])
        }
        _ => {
            let colon = line.find(':')?;
            (line[..colon].trim_end().to_string(), &line[colon..])
        }
    };
    let rest = rest.strip_prefix(':')?;
    (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')).then_some(key)
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Follow a symlinked profile so the atomic replace keeps the link.
fn resolve(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "profile.yaml".into());
    let tmp = dir.join(format!(".{name}.jobpilot-tmp"));
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "\
# MASTER PROFILE - comments must survive
basics:
  name: Jane Doe   # trailing comment
  email: jane@example.com

# Render order.
sections: [summary, skills]

application:
  # an old note inside the block
  work authorization: Authorized to work in Vietnam
  notice period: 2 weeks

# Everything below stays as is
skills:
  - category: Languages
    items: [Go,   Rust]
";

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn reads_the_section_and_tolerates_its_absence() {
        let answers = parse_answers(PROFILE).unwrap();
        assert_eq!(answers, pairs(&[("work authorization", "Authorized to work in Vietnam"), ("notice period", "2 weeks")]));
        assert!(parse_answers("basics:\n  name: x\n").unwrap().is_empty());
        assert!(parse_answers("application:\n").unwrap().is_empty());
        assert!(parse_answers("").unwrap().is_empty());
        assert!(parse_answers("application: [a, b]\n").is_err());
        // Numbers and booleans are answers too.
        assert_eq!(parse_answers("application:\n  years: 4\n  relocate: true\n").unwrap(), pairs(&[("years", "4"), ("relocate", "true")]));
    }

    #[test]
    fn replacing_the_block_leaves_the_rest_byte_for_byte() {
        let (updated, changed) = merge(
            PROFILE,
            &pairs(&[("Notice period?", "1 month"), ("Salary expectation", "negotiable: 3000 USD")]),
        )
        .unwrap();
        assert_eq!(changed, 2);
        let answers = parse_answers(&updated).unwrap();
        // Loose match replaced the value in place; the new question is appended.
        assert_eq!(
            answers,
            pairs(&[
                ("work authorization", "Authorized to work in Vietnam"),
                ("notice period", "1 month"),
                ("Salary expectation", "negotiable: 3000 USD"),
            ])
        );
        let before: Vec<&str> = PROFILE.split("application:").collect();
        let after: Vec<&str> = updated.split("application:").collect();
        assert_eq!(before[0], after[0], "everything before the block is untouched");
        let tail = "\n# Everything below stays as is\nskills:\n  - category: Languages\n    items: [Go,   Rust]\n";
        assert!(updated.ends_with(tail), "everything after the block is untouched:\n{updated}");
        assert!(updated.contains("# MASTER PROFILE - comments must survive"));
        assert!(updated.contains("name: Jane Doe   # trailing comment"));
        assert!(updated.contains("# Render order.\nsections: [summary, skills]\n"));
        assert_eq!(updated.matches("application:").count(), 1);
    }

    #[test]
    fn appends_a_block_when_there_is_none() {
        let original = "basics:\n  name: Jane\n# end comment\n";
        let (updated, changed) = merge(original, &pairs(&[("Sponsorship needed?", "No")])).unwrap();
        assert_eq!(changed, 1);
        assert!(updated.starts_with(original), "the original text is kept as a prefix:\n{updated}");
        assert!(updated.contains(HEADER));
        assert_eq!(parse_answers(&updated).unwrap(), pairs(&[("Sponsorship needed?", "No")]));

        // No trailing newline in the original, and an empty document.
        let (updated, _) = merge("basics:\n  name: Jane", &pairs(&[("a", "b")])).unwrap();
        assert!(updated.starts_with("basics:\n  name: Jane\n\n"));
        let (updated, _) = merge("", &pairs(&[("a", "b")])).unwrap();
        assert_eq!(parse_answers(&updated).unwrap(), pairs(&[("a", "b")]));
    }

    #[test]
    fn block_at_the_end_of_the_file_and_unchanged_answers() {
        let original = "basics:\n  name: Jane\napplication:\n  a: b\n";
        let (updated, changed) = merge(original, &pairs(&[("a", "b")])).unwrap();
        assert_eq!((updated.as_str(), changed), (original, 0), "identical answers do not rewrite anything");
        let (updated, changed) = merge(original, &pairs(&[("a", "c")])).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(updated, "basics:\n  name: Jane\napplication:\n  a: c\n");
    }

    #[test]
    fn awkward_values_round_trip() {
        let answers = pairs(&[
            ("Why do you want to work here?", "Because: it's \"great\"\nSecond line"),
            ("Expected salary", "# not a comment"),
            ("yes", "no"),
        ]);
        let (updated, _) = merge("basics:\n  name: J\n", &answers).unwrap();
        assert_eq!(parse_answers(&updated).unwrap(), answers);
    }

    #[test]
    fn finds_only_the_real_top_level_key() {
        let text = "basics:\n  application: nested is not the block\nsummary:\n  - application: x\n";
        assert_eq!(find_block(&text.split_inclusive('\n').collect::<Vec<_>>()), None);
        assert_eq!(top_level_key("application:\n").as_deref(), Some("application"));
        assert_eq!(top_level_key("application: {}  # none\n").as_deref(), Some("application"));
        assert_eq!(top_level_key("\"application\":\n").as_deref(), Some("application"));
        assert_eq!(top_level_key("application_x: 1\n"), Some("application_x".into()));
        assert_eq!(top_level_key("  application:\n"), None);
        assert_eq!(top_level_key("# application:\n"), None);
    }

    #[test]
    fn remember_edits_a_copy_of_the_real_profile_and_nothing_else() {
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../profile.yaml");
        let Ok(original) = std::fs::read_to_string(&real) else { return };
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("profile.yaml");
        std::fs::write(&copy, &original).unwrap();

        let changed = remember(&copy, &pairs(&[("Notice period", "2 weeks")])).unwrap();
        assert_eq!(changed, 1);
        let updated = std::fs::read_to_string(&copy).unwrap();
        assert!(updated.starts_with(&original), "everything that was there is kept verbatim");
        assert_eq!(read_answers(&copy).unwrap(), pairs(&[("Notice period", "2 weeks")]));
        // The original text still parses to the same CV facts.
        let before = crate::engine::pipeline::Cv::parse(&original).unwrap();
        let after = crate::engine::pipeline::Cv::parse(&updated).unwrap();
        assert_eq!(before, after);
        // The real file was only read.
        assert_eq!(std::fs::read_to_string(&real).unwrap(), original);
    }

    #[test]
    fn normalize_key_ignores_case_and_punctuation() {
        assert_eq!(normalize_key("  Notice Period? *"), "notice period");
        assert_eq!(normalize_key("work_authorization"), "work authorization");
    }
}
