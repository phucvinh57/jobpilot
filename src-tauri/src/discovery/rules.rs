//! Pure rules of this context, unit-tested here: what the link box holds, and which dropped files
//! are job descriptions. Rules that need no file system (ordering, labels, the primary action,
//! skills matching, the add queue) live in the web app's `lib/discovery/` with their vitest tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::dto::LeadInput;

/// File types accepted by drag and drop.
pub const JD_EXTENSIONS: [&str; 5] = ["md", "txt", "html", "htm", "pdf"];

/// The first look at the daily-search setting after the app starts, and the gap between looks after
/// that.
pub const DAILY_FIRST_CHECK: Duration = Duration::from_secs(60);
pub const DAILY_CHECK_EVERY: Duration = Duration::from_secs(15 * 60);

/// What the user typed or pasted into the link box: a link, a path to a JD file, or the JD text.
/// `None` for blank input.
pub fn parse_input(raw: &str) -> Option<LeadInput> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let single_token = !raw.contains(char::is_whitespace);
    if single_token && has_http_scheme(raw) {
        return Some(LeadInput::Url(raw.to_string()));
    }
    if let Some(path) = as_file_path(raw) {
        return Some(LeadInput::File(path.to_string_lossy().into_owned()));
    }
    if single_token && looks_like_bare_url(raw) {
        return Some(LeadInput::Url(format!("https://{raw}")));
    }
    Some(LeadInput::Text(raw.to_string()))
}

fn has_http_scheme(s: &str) -> bool {
    let lower = s.get(..8).unwrap_or(s).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// `jobs.example.com/careers/42`: a host with a dot, then a path.
fn looks_like_bare_url(s: &str) -> bool {
    let Some((host, _)) = s.split_once('/') else {
        return false;
    };
    host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// The host of a source's address (`https://boards.greenhouse.io/acme?token=x` -> `boards.greenhouse.io`);
/// blank for a blank address. The web app never gets more than this.
pub fn host_of(url: &str) -> String {
    let url = url.trim();
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .to_string()
}

/// A pasted path (`/home/me/jd.pdf`, optionally quoted or a `file://` URI) that is a file.
fn as_file_path(raw: &str) -> Option<PathBuf> {
    let unquoted = raw.trim_matches(|c| c == '"' || c == '\'');
    let path = match unquoted.strip_prefix("file://") {
        Some(rest) => PathBuf::from(percent_decode(rest)),
        None => PathBuf::from(unquoted),
    };
    (path.is_absolute() && is_jd_file(&path)).then_some(path)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A file of a supported posting type. Pasted paths and dropped files both pass through here, so
/// no other file (a key, a binary) is ever read as a posting.
fn is_jd_file(path: &Path) -> bool {
    path.is_file() && extension(path).is_some_and(|e| JD_EXTENSIONS.contains(&e.as_str()))
}

/// The first dropped file that is a supported JD file; an error line when there is none.
pub fn file_input(paths: &[PathBuf]) -> Result<LeadInput, String> {
    if paths.is_empty() {
        return Err("Nothing was dropped.".into());
    }
    for path in paths {
        if is_jd_file(path) {
            return Ok(LeadInput::File(path.to_string_lossy().into_owned()));
        }
    }
    let first = &paths[0];
    let name = first
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Err(match extension(first) {
        _ if first.is_dir() => format!("{name} is a folder: drop a .md, .txt, .html or .pdf file."),
        Some(ext) => {
            format!("{name}: .{ext} files are not supported (use .md, .txt, .html or .pdf).")
        }
        None => format!("{name}: unknown file type (use .md, .txt, .html or .pdf)."),
    })
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{file_input, host_of, parse_input, LeadInput};

    #[test]
    fn a_source_address_is_cut_to_its_host() {
        assert_eq!(
            host_of("https://boards.greenhouse.io/acme?token=secret"),
            "boards.greenhouse.io"
        );
        assert_eq!(host_of("boards.greenhouse.io/acme"), "boards.greenhouse.io");
        assert_eq!(host_of("https://user:pass@feeds.test/jobs"), "feeds.test");
        assert_eq!(host_of("https://feeds.test?key=1"), "feeds.test");
        assert_eq!(host_of("  "), "");
    }

    #[test]
    fn a_pasted_path_is_a_file_only_when_it_is_a_posting_type() {
        let dir = std::env::temp_dir().join(format!("jobpilot-rules-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("id_rsa");
        let jd = dir.join("jd.md");
        std::fs::write(&key, "secret").unwrap();
        std::fs::write(&jd, "# Role").unwrap();
        let pasted_key = parse_input(&key.to_string_lossy());
        let pasted_jd = parse_input(&jd.to_string_lossy());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            !matches!(pasted_key, Some(LeadInput::File(_))),
            "a key file is never read as a posting"
        );
        assert!(matches!(pasted_jd, Some(LeadInput::File(_))));
    }

    fn url(input: Option<LeadInput>) -> String {
        match input {
            Some(LeadInput::Url(u)) => u,
            other => panic!("not a URL: {other:?}"),
        }
    }

    #[test]
    fn links_files_and_text_are_told_apart() {
        assert_eq!(
            url(parse_input("  https://acme.test/jobs/1 ")),
            "https://acme.test/jobs/1"
        );
        assert_eq!(url(parse_input("HTTP://ACME.test/x")), "HTTP://ACME.test/x");
        assert_eq!(
            url(parse_input("jobs.acme.test/careers/42")),
            "https://jobs.acme.test/careers/42"
        );
        assert!(parse_input("   ").is_none());

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("my jd.md");
        std::fs::write(&file, "# JD").unwrap();
        let path = file.display().to_string();
        let quoted = format!("'{path}'");
        assert_eq!(parse_input(&quoted), Some(LeadInput::File(path.clone())));
        let uri = format!("file://{}", path.replace(' ', "%20"));
        assert_eq!(parse_input(&uri), Some(LeadInput::File(path)));
        // A path that does not exist is just text.
        assert!(matches!(
            parse_input("/no/such/jd.md"),
            Some(LeadInput::Text(_))
        ));

        // A sentence that mentions a link is a JD, not a link.
        let text = "We are hiring a Rust engineer. Apply at https://acme.test/apply";
        assert_eq!(parse_input(text), Some(LeadInput::Text(text.to_string())));
        assert!(matches!(
            parse_input("Senior.Engineer"),
            Some(LeadInput::Text(_))
        ));
    }

    #[test]
    fn dropped_files_need_a_supported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, "x").unwrap();
            p
        };
        let (md, pdf, png) = (make("a.MD"), make("b.pdf"), make("c.png"));
        let file = |p: &std::path::Path| LeadInput::File(p.to_string_lossy().into_owned());
        assert_eq!(file_input(std::slice::from_ref(&md)), Ok(file(&md)));
        assert_eq!(file_input(&[png.clone(), pdf.clone()]), Ok(file(&pdf)));
        assert!(file_input(&[png]).unwrap_err().contains(".png"));
        assert!(file_input(&[dir.path().to_path_buf()])
            .unwrap_err()
            .contains("folder"));
        assert!(file_input(&[]).is_err());
        assert!(file_input(&[dir.path().join("missing.md")]).is_err());
    }

    #[test]
    fn a_refused_drop_says_what_to_drop_instead() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("README");
        std::fs::write(&bare, "x").unwrap();
        assert!(file_input(&[bare])
            .unwrap_err()
            .contains("unknown file type"));
        let folder = file_input(&[dir.path().to_path_buf()]).unwrap_err();
        assert!(folder.contains(".pdf"), "{folder}");
    }
}
