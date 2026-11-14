//! Spotting the hiring system behind a careers page: a page that embeds a Greenhouse board, links
//! to Lever postings or loads a Workday widget lists its jobs there, and the board's own source
//! reads them better than the page does.
//!
//! What a page names is **only a suggestion**: the app never reads a detected board by itself. A
//! [`BoardHint`] is shown to the user (Settings > Job sources, or the status of a career-page run),
//! who may add it as a source. Every hint is a board address built by code from the pattern table
//! below and checked by [`check_source`], the same check as a source the user types, so a bad
//! name is never suggested.
//!
//! | Address | Kind |
//! |---------|------|
//! | `boards.greenhouse.io/<slug>`, `job-boards.greenhouse.io/<slug>`, `boards.greenhouse.io/embed/job_board[/js]?for=<slug>` | `greenhouse` |
//! | `jobs.lever.co/<slug>` | `lever` |
//! | `jobs.ashbyhq.com/<slug>` | `ashby` |
//! | `jobs.smartrecruiters.com/<Company>` | `smartrecruiters` |
//! | `<company>.recruitee.com` | `recruitee` |
//! | `<company>.jobs.personio.de` / `.com` | `personio` |
//! | `<tenant>.wd<N>.myworkdayjobs.com/[<locale>/]<site>` | `workday` |

use std::sync::OnceLock;

use regex::Regex;
use url::Url;

use super::web::{resolve, Page};
use super::{board_host, check_source, is_board_name, personio, recruitee, workday};
use crate::engine::discover::sources::JOB_SOURCE_KINDS;

/// A board a careers page points to: what to add as a source of `kind` (the input of
/// [`check_source`], which it passes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardHint {
    pub kind: &'static str,
    pub input: String,
}

impl BoardHint {
    /// The kind's label as the UI shows it (`Greenhouse`).
    pub fn label(&self) -> &'static str {
        JOB_SOURCE_KINDS.iter().find(|(k, _)| *k == self.kind).map_or(self.kind, |(_, label)| label)
    }
}

/// The registrable domains of each board kind.
const BOARD_DOMAINS: [(&str, &[&str]); 7] = [
    ("greenhouse", &["greenhouse.io"]),
    ("lever", &["lever.co"]),
    ("ashby", &["ashbyhq.com"]),
    ("smartrecruiters", &["smartrecruiters.com"]),
    ("recruitee", &["recruitee.com"]),
    ("personio", &["personio.de", "personio.com"]),
    ("workday", &["myworkdayjobs.com"]),
];

/// The board kind whose domain `host` is on (`boards.greenhouse.io` -> `greenhouse`); `None` for
/// any other host, look-alikes (`greenhouse.io.evil.test`) included.
pub fn kind_for_host(host: &str) -> Option<&'static str> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    BOARD_DOMAINS
        .iter()
        .find(|(_, domains)| domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}"))))
        .map(|(kind, _)| *kind)
}

/// Most hints taken from one page.
const MAX_HINTS: usize = 10;

/// The boards that `html` (read at `base`) points to, through `<a href>`, `<iframe src>` and
/// `<script src>`, in page order, each once. Relative addresses are resolved against `base`.
pub fn detect_boards(html: &str, base: &Url) -> Vec<BoardHint> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| {
        Regex::new(r#"(?is)<(a|iframe|script)\b[^>]*?\s(href|src)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).expect("static regex")
    });
    let mut out: Vec<BoardHint> = Vec::new();
    for caps in tag.captures_iter(html) {
        let (name, attr) = (caps[1].to_ascii_lowercase(), caps[2].to_ascii_lowercase());
        if (name == "a") != (attr == "href") {
            continue;
        }
        let raw = caps.get(3).or_else(|| caps.get(4)).or_else(|| caps.get(5)).map(|m| m.as_str().trim()).unwrap_or_default();
        let Some(url) = resolve(base, &raw.replace("&amp;", "&")) else { continue };
        if let Some(hint) = hint_for(&url) {
            if !out.contains(&hint) {
                out.push(hint);
                if out.len() >= MAX_HINTS {
                    break;
                }
            }
        }
    }
    out
}

/// [`detect_boards`] of a page that was read, plus the page's own address when it ended on a board
/// (a careers page that redirects to its Greenhouse board).
pub fn page_hints(page: &Page) -> Vec<BoardHint> {
    let Ok(base) = Url::parse(&page.url) else { return Vec::new() };
    let mut hints: Vec<BoardHint> = hint_for(&base).into_iter().collect();
    for hint in detect_boards(&page.html, &base) {
        if !hints.contains(&hint) && hints.len() < MAX_HINTS {
            hints.push(hint);
        }
    }
    hints
}

/// Labels on a board's domain that are the vendor's own pages, not a company's board.
const VENDOR_LABELS: [&str; 8] = ["www", "app", "api", "help", "blog", "support", "status", "docs"];

/// The board behind one address, built from the pattern table and checked by [`check_source`]. The
/// board name is validated before it is put into the address, and the address must read back to
/// that same name, so a name carrying `?`, `#` or `/` (decoded from `for=acme%3Ffor%3Devil`) can
/// never make the hint name one board and add another.
fn hint_for(url: &Url) -> Option<BoardHint> {
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    let first = url.path_segments().and_then(|mut s| s.find(|p| !p.is_empty()).map(str::to_string));
    let named = |slug: Option<String>| slug.filter(|s| is_board_name(s));
    // (kind, address to add, the name the address must read back to)
    let (kind, input, name) = match kind_for_host(&host)? {
        "greenhouse" if host == "boards.greenhouse.io" || host == "job-boards.greenhouse.io" => {
            let slug = match first.as_deref() {
                // `embed/job_board?for=<slug>` and `embed/job_board/js?for=<slug>`
                Some("embed") => named(url.query_pairs().find(|(k, _)| k == "for").map(|(_, v)| v.into_owned()))?,
                _ => named(first)?,
            };
            ("greenhouse", format!("{host}/{slug}"), slug)
        }
        kind @ ("lever" | "ashby" | "smartrecruiters")
            if matches!((kind, host.as_str()), ("lever", "jobs.lever.co") | ("ashby", "jobs.ashbyhq.com") | ("smartrecruiters", "jobs.smartrecruiters.com")) =>
        {
            let slug = named(first)?;
            (kind, format!("{host}/{slug}"), slug)
        }
        kind @ ("recruitee" | "personio") => {
            let hosts = if kind == "recruitee" { recruitee::HOSTS } else { personio::HOSTS };
            let (name, suffix) = board_host(&host, hosts).ok()?;
            if VENDOR_LABELS.contains(&name.as_str()) {
                return None;
            }
            (kind, format!("{name}.{suffix}"), name)
        }
        "workday" => {
            let board = workday::Board::parse(url.as_str()).ok()?;
            let input = format!("https://{}/{}", board.host, board.site);
            if workday::Board::parse(&input).ok()? != board {
                return None;
            }
            ("workday", input, board.tenant)
        }
        _ => return None,
    };
    let (_, read_back) = check_source(kind, &input).ok()?;
    (read_back == name).then_some(BoardHint { kind, input })
}

/// The status of a career-page run that found no postings on a page that points to a board.
pub fn no_postings_status(hints: &[BoardHint]) -> Option<String> {
    let first = hints.first()?;
    let label = first.label();
    Some(format!("No postings on the page. It links to a {label} board: add it as a {label} source"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMBED: &str = include_str!("../fixtures/career_page_embed.html");

    fn base() -> Url {
        Url::parse("https://acme.test/careers").unwrap()
    }

    fn found(html: &str) -> Vec<(&'static str, String)> {
        detect_boards(html, &base()).into_iter().map(|h| (h.kind, h.input)).collect()
    }

    fn one(href: &str) -> Option<(&'static str, String)> {
        found(&format!(r#"<a href="{href}">Jobs</a>"#)).into_iter().next()
    }

    #[test]
    fn every_pattern_gives_its_board() {
        let gh = |s: &str| Some(("greenhouse", s.to_string()));
        assert_eq!(one("https://boards.greenhouse.io/acme"), gh("boards.greenhouse.io/acme"));
        assert_eq!(one("https://boards.greenhouse.io/acme/jobs/4011001?gh_jid=1"), gh("boards.greenhouse.io/acme"));
        assert_eq!(one("https://job-boards.greenhouse.io/acme"), gh("job-boards.greenhouse.io/acme"));
        assert_eq!(one("https://boards.greenhouse.io/embed/job_board?for=acme&amp;b=https://acme.test"), gh("boards.greenhouse.io/acme"));
        assert_eq!(one("https://boards.greenhouse.io/embed/job_board/js?for=acme"), gh("boards.greenhouse.io/acme"));
        assert_eq!(one("https://jobs.lever.co/acme/0b1c2d3e-aaaa-bbbb-cccc-123456789012"), Some(("lever", "jobs.lever.co/acme".into())));
        assert_eq!(one("https://jobs.ashbyhq.com/acme"), Some(("ashby", "jobs.ashbyhq.com/acme".into())));
        assert_eq!(one("https://jobs.smartrecruiters.com/AcmeGroup/7440001"), Some(("smartrecruiters", "jobs.smartrecruiters.com/AcmeGroup".into())));
        assert_eq!(one("https://acme.recruitee.com/o/backend-engineer"), Some(("recruitee", "acme.recruitee.com".into())));
        assert_eq!(one("https://acme.jobs.personio.de/job/1"), Some(("personio", "acme.jobs.personio.de".into())));
        assert_eq!(one("https://ACME.jobs.personio.com/"), Some(("personio", "acme.jobs.personio.com".into())));
        assert_eq!(one("https://acme.wd5.myworkdayjobs.com/en-US/External/job/x"), Some(("workday", "https://acme.wd5.myworkdayjobs.com/External".into())));
        assert_eq!(one("//jobs.lever.co/acme"), Some(("lever", "jobs.lever.co/acme".into())), "a protocol-relative link");
    }

    #[test]
    fn look_alikes_and_vendor_pages_are_refused() {
        for href in [
            "https://boards.greenhouse.io.evil.test/acme",
            "https://evil.test/boards.greenhouse.io/acme",
            "https://evil.test/?u=https://boards.greenhouse.io/acme",
            "https://boards.greenhouse.io@evil.test/acme",
            "https://user@boards.greenhouse.io/acme",
            "https://boards.greenhouse.io:8443/acme",
            "https://notgreenhouse.io/acme",
            "https://api.greenhouse.io/acme",
            "https://www.greenhouse.io/careers",
            "https://boards.greenhouse.io/",
            "https://boards.greenhouse.io/embed/job_board",
            "https://boards.greenhouse.io/embed/job_board?for=..",
            "https://jobs.lever.co/",
            "https://lever.co/acme",
            "https://jobs.lever.co.evil.test/acme",
            "https://www.recruitee.com/",
            "https://a.b.recruitee.com/",
            "https://recruitee.com.evil.test/",
            "https://www.personio.de/",
            "https://acme.personio.de/",
            "https://www.myworkdayjobs.com/acme",
            "https://acme.wd5.myworkdayjobs.com/",
            "https://acme.wd5.myworkdayjobs.com.evil.test/External",
            "ftp://jobs.lever.co/acme",
            "https://boards.greenhouse.io/embed/job_board?for=acme%3Ffor%3Devil",
            "https://boards.greenhouse.io/embed/job_board?for=acme%23x",
            "https://boards.greenhouse.io/embed/job_board?for=acme%2Fevil",
            "https://boards.greenhouse.io/embed/job_board?for=evil.test%2Fx",
            "https://boards.greenhouse.io/acme%2F..%2Fevil",
            "https://jobs.lever.co/acme%3Fx%3Dy",
            "https://jobs.ashbyhq.com/acme%23x",
            "https://jobs.smartrecruiters.com/acme%2Fevil",
            "javascript:location='https://jobs.lever.co/acme'",
        ] {
            assert_eq!(one(href), None, "{href}");
        }
    }

    #[test]
    fn hints_pass_the_source_check() {
        for hint in detect_boards(EMBED, &base()) {
            assert!(check_source(hint.kind, &hint.input).is_ok(), "{hint:?}");
        }
        // A slug the check refuses is never suggested.
        assert_eq!(one("https://jobs.lever.co/a%20b"), None);
        assert_eq!(one("https://jobs.ashbyhq.com/..."), None);
    }

    #[test]
    fn the_fixture_gives_its_greenhouse_iframe_and_lever_link_once_each() {
        assert_eq!(
            found(EMBED),
            [("greenhouse", "boards.greenhouse.io/acme".to_string()), ("lever", "jobs.lever.co/acme".to_string())]
        );
    }

    #[test]
    fn only_links_iframes_and_scripts_count() {
        let html = r#"
            <link rel="preconnect" href="https://jobs.lever.co/acme">
            <img src="https://jobs.ashbyhq.com/acme/logo.png">
            <p>See https://boards.greenhouse.io/acme</p>
            <a data-href="https://jobs.lever.co/beta" href="/careers">x</a>
            <script src='https://boards.greenhouse.io/embed/job_board/js?for=gamma'></script>
            <iframe title="Jobs" src=https://jobs.ashbyhq.com/delta></iframe>
            <a href="https://jobs.lever.co/epsilon">Lever</a>"#;
        assert_eq!(
            found(html),
            [
                ("greenhouse", "boards.greenhouse.io/gamma".to_string()),
                ("ashby", "jobs.ashbyhq.com/delta".to_string()),
                ("lever", "jobs.lever.co/epsilon".to_string()),
            ]
        );
        let many: String = (0..30).map(|i| format!(r#"<a href="https://jobs.lever.co/c{i}">x</a>"#)).collect();
        assert_eq!(found(&many).len(), MAX_HINTS);
    }

    #[test]
    fn a_page_that_ended_on_a_board_suggests_that_board() {
        let page = Page { url: "https://boards.greenhouse.io/acme".into(), html: r#"<a href="https://jobs.lever.co/acme">x</a>"#.into() };
        let hints: Vec<_> = page_hints(&page).into_iter().map(|h| h.kind).collect();
        assert_eq!(hints, ["greenhouse", "lever"]);
        assert!(page_hints(&Page { url: "not a url".into(), html: String::new() }).is_empty());
    }

    #[test]
    fn hosts_name_their_board_kind() {
        for (host, kind) in [
            ("boards.greenhouse.io", "greenhouse"),
            ("greenhouse.io", "greenhouse"),
            ("jobs.lever.co", "lever"),
            ("jobs.ashbyhq.com", "ashby"),
            ("jobs.smartrecruiters.com", "smartrecruiters"),
            ("acme.recruitee.com", "recruitee"),
            ("acme.jobs.personio.de", "personio"),
            ("acme.jobs.personio.com.", "personio"),
            ("acme.wd5.myworkdayjobs.com", "workday"),
        ] {
            assert_eq!(kind_for_host(host), Some(kind), "{host}");
        }
        for host in ["greenhouse.io.evil.test", "notlever.co", "acme.test", "linkedin.com", ""] {
            assert_eq!(kind_for_host(host), None, "{host}");
        }
    }

    #[test]
    fn the_run_status_names_the_board() {
        assert_eq!(no_postings_status(&[]), None);
        let hints = [BoardHint { kind: "greenhouse", input: "boards.greenhouse.io/acme".into() }, BoardHint { kind: "lever", input: "jobs.lever.co/acme".into() }];
        assert_eq!(
            no_postings_status(&hints).as_deref(),
            Some("No postings on the page. It links to a Greenhouse board: add it as a Greenhouse source")
        );
        assert_eq!(BoardHint { kind: "smartrecruiters", input: String::new() }.label(), "SmartRecruiters");
    }
}
