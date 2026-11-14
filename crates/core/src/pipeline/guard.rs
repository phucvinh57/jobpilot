//! Fact guard: a deterministic check that a tailored CV only states facts the master profile has.
//!
//! The model writes the CV; this is the safety net. It checks identity (name, contact, links),
//! structure (every employer + title + dates, project, school, certification and language exists
//! in the profile), every number a reader sees, and every skill item. It does not judge prose:
//! rewording a bullet is allowed, inventing a number or an employer is not.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::OnceLock;

use regex::Regex;

use super::cv::Cv;
use crate::Core;

/// One broken rule: where (`experience[1].company`) and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub path: String,
    pub message: String,
}

impl Violation {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { path: path.into(), message: message.into() }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Terms the guard treats as the same skill (all lower case).
const SYNONYMS: &[&[&str]] = &[
    &["k8s", "kubernetes"],
    &["js", "javascript"],
    &["ts", "typescript"],
    &["postgres", "postgresql"],
    &["golang", "go"],
    &["mongo", "mongodb"],
    &["node", "nodejs", "node.js"],
];

/// Check `cv` against the master `profile`. Empty result = the CV is clean.
pub fn guard(profile: &Cv, cv: &Cv) -> Vec<Violation> {
    let mut out = Vec::new();
    check_basics(profile, cv, &mut out);
    check_experience(profile, cv, &mut out);
    check_projects(profile, cv, &mut out);
    check_education(profile, cv, &mut out);
    check_certifications(profile, cv, &mut out);
    check_languages(profile, cv, &mut out);
    let corpus = profile.corpus();
    check_numbers(&corpus, cv, &mut out);
    check_skills(&corpus, cv, &mut out);
    out
}

/// Render violations as a bullet list (for events, errors and the repair prompt).
pub fn format_violations(violations: &[Violation]) -> String {
    violations.iter().map(|v| format!("- {v}")).collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------- normalisation

/// Case and whitespace-insensitive form.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A link without scheme, `www.` or trailing slash.
fn norm_url(s: &str) -> String {
    let s = norm(s);
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(&s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.trim_end_matches('/').to_string()
}

fn norm_phone(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect()
}

// ---------------------------------------------------------------- identity

fn check_basics(profile: &Cv, cv: &Cv, out: &mut Vec<Violation>) {
    let (p, c) = (&profile.basics, &cv.basics);
    if norm(&c.name) != norm(&p.name) {
        out.push(Violation::new("basics.name", format!("must be `{}` (the profile name), found `{}`", p.name, c.name)));
    }
    let mut same = |path: &str, pv: &str, cv: &str, f: fn(&str) -> String| {
        if !cv.trim().is_empty() && f(cv) != f(pv) {
            let shown = if pv.trim().is_empty() { "empty in the profile".to_string() } else { format!("`{pv}`") };
            out.push(Violation::new(path, format!("must match the profile ({shown}), found `{cv}`")));
        }
    };
    same("basics.email", &p.email, &c.email, norm);
    same("basics.phone", &p.phone, &c.phone, norm_phone);
    same("basics.github", &p.github, &c.github, norm_url);
    same("basics.linkedin", &p.linkedin, &c.linkedin, norm_url);
    same("basics.website", &p.website, &c.website, norm_url);

    if !c.location.trim().is_empty() {
        let known: BTreeSet<String> = std::iter::once(p.location.as_str())
            .chain(profile.experience.iter().map(|e| e.location.as_str()))
            .chain(profile.education.iter().map(|e| e.location.as_str()))
            .map(norm)
            .filter(|l| !l.is_empty())
            .collect();
        if !known.contains(&norm(&c.location)) {
            out.push(Violation::new(
                "basics.location",
                format!("`{}` is not a location in the profile (profile: `{}`)", c.location, p.location),
            ));
        }
    }
}

// ---------------------------------------------------------------- structure

/// For a non-empty CV value, it must equal the profile's (case/whitespace-insensitive).
fn field_matches(path: &str, label: &str, cv_value: &str, profile_value: &str, out: &mut Vec<Violation>) {
    if cv_value.trim().is_empty() || norm(cv_value) == norm(profile_value) {
        return;
    }
    out.push(Violation::new(path, format!("{label} `{cv_value}` differs from the profile (`{profile_value}`)")));
}

fn url_matches(path: &str, cv_value: &str, profile_value: &str, out: &mut Vec<Violation>) {
    if !cv_value.trim().is_empty() && norm_url(cv_value) != norm_url(profile_value) {
        out.push(Violation::new(path, format!("url `{cv_value}` differs from the profile (`{profile_value}`)")));
    }
}

fn check_experience(profile: &Cv, cv: &Cv, out: &mut Vec<Violation>) {
    for (i, e) in cv.experience.iter().enumerate() {
        let at = |f: &str| format!("experience[{i}].{f}");
        let same_company: Vec<_> = profile.experience.iter().filter(|p| norm(&p.company) == norm(&e.company)).collect();
        if same_company.is_empty() {
            out.push(Violation::new(at("company"), format!("`{}` is not an employer in the profile", e.company)));
            continue;
        }
        let exact = same_company.iter().find(|p| {
            norm(&p.position) == norm(&e.position) && norm(&p.start) == norm(&e.start) && norm(&p.end) == norm(&e.end)
        });
        match exact {
            Some(p) => {
                field_matches(&at("location"), "location", &e.location, &p.location, out);
                url_matches(&at("url"), &e.url, &p.url, out);
            }
            None => {
                let known = same_company
                    .iter()
                    .map(|p| format!("`{}` {} - {}", p.position, p.start, p.end))
                    .collect::<Vec<_>>()
                    .join("; ");
                out.push(Violation::new(
                    at("position"),
                    format!(
                        "`{}` {} - {} at {} does not match the profile ({known})",
                        e.position, e.start, e.end, e.company
                    ),
                ));
            }
        }
    }
}

fn check_projects(profile: &Cv, cv: &Cv, out: &mut Vec<Violation>) {
    for (i, p) in cv.projects.iter().enumerate() {
        let at = |f: &str| format!("projects[{i}].{f}");
        let Some(src) = profile.projects.iter().find(|s| norm(&s.name) == norm(&p.name)) else {
            out.push(Violation::new(at("name"), format!("`{}` is not a project in the profile", p.name)));
            continue;
        };
        field_matches(&at("role"), "role", &p.role, &src.role, out);
        field_matches(&at("start"), "start", &p.start, &src.start, out);
        field_matches(&at("end"), "end", &p.end, &src.end, out);
        url_matches(&at("url"), &p.url, &src.url, out);
    }
}

fn check_education(profile: &Cv, cv: &Cv, out: &mut Vec<Violation>) {
    for (i, e) in cv.education.iter().enumerate() {
        let at = |f: &str| format!("education[{i}].{f}");
        let Some(src) = profile.education.iter().find(|s| norm(&s.institution) == norm(&e.institution)) else {
            out.push(Violation::new(at("institution"), format!("`{}` is not a school in the profile", e.institution)));
            continue;
        };
        field_matches(&at("degree"), "degree", &e.degree, &src.degree, out);
        field_matches(&at("location"), "location", &e.location, &src.location, out);
        field_matches(&at("gpa"), "gpa", &e.gpa, &src.gpa, out);
        field_matches(&at("start"), "start", &e.start, &src.start, out);
        field_matches(&at("end"), "end", &e.end, &src.end, out);
    }
}

fn check_certifications(profile: &Cv, cv: &Cv, out: &mut Vec<Violation>) {
    for (i, c) in cv.certifications.iter().enumerate() {
        let at = |f: &str| format!("certifications[{i}].{f}");
        let Some(src) = profile.certifications.iter().find(|s| norm(&s.name) == norm(&c.name)) else {
            out.push(Violation::new(at("name"), format!("`{}` is not a certification in the profile", c.name)));
            continue;
        };
        field_matches(&at("issuer"), "issuer", &c.issuer, &src.issuer, out);
        field_matches(&at("date"), "date", &c.date, &src.date, out);
    }
}

fn check_languages(profile: &Cv, cv: &Cv, out: &mut Vec<Violation>) {
    for (i, l) in cv.languages.iter().enumerate() {
        let at = |f: &str| format!("languages[{i}].{f}");
        let Some(src) = profile.languages.iter().find(|s| norm(&s.language) == norm(&l.language)) else {
            out.push(Violation::new(at("language"), format!("`{}` is not a language in the profile", l.language)));
            continue;
        };
        field_matches(&at("fluency"), "fluency", &l.fluency, &src.fluency, out);
    }
}

// ---------------------------------------------------------------- numbers

/// Digit tokens of `text`: `5000+` -> `5000`, `~13MB` -> `13`, `1,000` -> `1000`, `3.5` -> `3.5`.
pub fn number_tokens(text: &str) -> Vec<String> {
    static NUMBER: OnceLock<Regex> = OnceLock::new();
    let re = NUMBER.get_or_init(|| Regex::new(r"\d+(?:[.,]\d+)*").expect("static regex"));
    re.find_iter(text).map(|m| m.as_str().replace(',', "")).collect()
}

fn check_numbers(profile_corpus: &str, cv: &Cv, out: &mut Vec<Violation>) {
    let known: BTreeSet<String> = number_tokens(profile_corpus).into_iter().collect();
    for (path, text) in cv.prose() {
        for token in number_tokens(text) {
            if !known.contains(&token) {
                out.push(Violation::new(path.clone(), format!("number `{token}` does not appear in the profile")));
            }
        }
    }
}

// ---------------------------------------------------------------- skills

/// Occurrence of `needle` in `hay` not touching other letters/digits (for 1-2 letter terms like `Go`).
fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn term_in_corpus(corpus: &str, term: &str) -> bool {
    let term = norm(term);
    if term.is_empty() {
        return true;
    }
    let mut candidates = vec![term.clone()];
    for group in SYNONYMS.iter().filter(|g| g.contains(&term.as_str())) {
        candidates.extend(group.iter().map(|s| s.to_string()));
    }
    candidates
        .iter()
        .any(|c| if c.chars().count() <= 2 { contains_word(corpus, c) } else { corpus.contains(c.as_str()) })
}

fn check_skills(profile_corpus: &str, cv: &Cv, out: &mut Vec<Violation>) {
    for (i, group) in cv.skills.iter().enumerate() {
        for (j, item) in group.items.iter().enumerate() {
            if !term_in_corpus(profile_corpus, item) {
                out.push(Violation::new(
                    format!("skills[{i}].items[{j}]"),
                    format!("skill `{item}` does not appear in the profile"),
                ));
            }
        }
    }
}

impl Core {
    /// Does `cv_yaml` pass the fact guard against `profile.yaml`? `false` when it cannot be told
    /// (an unreadable or invalid profile, a CV that does not parse). Blocking file read.
    pub fn cv_is_traced(&self, cv_yaml: &str) -> bool {
        let Ok(profile_text) = std::fs::read_to_string(self.profile_path()) else { return false };
        match (Cv::parse(&profile_text), Cv::parse(cv_yaml)) {
            (Ok(profile), Ok(cv)) => guard(&profile, &cv).is_empty(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = include_str!("../../../../profile.yaml");
    const EXAMPLE: &str = include_str!("../../../../assets/prompts/example-cv.yaml");

    fn profile() -> Cv {
        Cv::parse(PROFILE).unwrap()
    }

    fn check(yaml: &str) -> Vec<Violation> {
        guard(&profile(), &Cv::parse(yaml).unwrap())
    }

    fn messages(v: &[Violation]) -> String {
        format_violations(v)
    }

    #[test]
    fn profile_and_example_parse() {
        let p = profile();
        assert_eq!(p.basics.name, "Vinh Nguyen Phuc");
        assert_eq!(p.experience.len(), 4);
        assert_eq!(p.experience[1].groups.len(), 4);
        assert!(p.experience[3].hidden);
        assert_eq!(p.projects[0].name, "Biopass");
        assert_eq!(p.basics.phone, "0373395726");
        let ex = Cv::parse(EXAMPLE).unwrap();
        assert_eq!(ex.sections.len(), 6);
        assert_eq!(ex.titles.len(), 0);
    }

    #[test]
    fn real_example_passes_against_real_profile() {
        let v = check(EXAMPLE);
        assert!(v.is_empty(), "the few-shot example must satisfy the guard:\n{}", messages(&v));
    }

    #[test]
    fn the_profile_is_valid_against_itself() {
        let v = guard(&profile(), &profile());
        assert!(v.is_empty(), "{}", messages(&v));
    }

    #[test]
    fn invented_employer_is_rejected() {
        let v = check(&EXAMPLE.replace("company: ST Engineering", "company: Globex Corporation"));
        assert!(
            v.iter().any(|v| v.path == "experience[0].company" && v.message.contains("Globex")),
            "{}",
            messages(&v)
        );
    }

    #[test]
    fn changed_title_or_dates_are_rejected() {
        let v = check(&EXAMPLE.replace("position: Backend Engineer", "position: Staff Engineer"));
        assert!(v.iter().any(|v| v.path == "experience[2].position"), "{}", messages(&v));
        let v = check(&EXAMPLE.replace("start: Oct 2022", "start: Oct 2020"));
        assert!(v.iter().any(|v| v.path == "experience[2].position"), "{}", messages(&v));
        // case and whitespace do not matter
        let v = check(&EXAMPLE.replace("company: ConcreteAI", "company:   concreteai  "));
        assert!(v.is_empty(), "{}", messages(&v));
    }

    #[test]
    fn invented_number_is_rejected() {
        let v = check(&EXAMPLE.replace("96GB RAM", "960GB RAM"));
        assert!(v.iter().any(|v| v.message.contains("`960`")), "{}", messages(&v));
        let v = check(&EXAMPLE.replace("multiple projects", "17 projects"));
        assert!(v.iter().any(|v| v.message.contains("`17`")), "{}", messages(&v));
    }

    #[test]
    fn number_tokens_are_normalised() {
        assert_eq!(
            number_tokens("5000+ nodes, ~13MB, 4+ years, 1,000 users, v3.5 and S3"),
            ["5000", "13", "4", "1000", "3.5", "3"]
        );
        // `5000+`, `~13MB`, `4+` from the profile are accepted in a CV
        let cv = "basics: {name: Vinh Nguyen Phuc}\nsummary:\n  - Managed 5000+ computers in a ~13MB runtime with 4+ years.\n";
        assert!(check(cv).is_empty());
        let cv = "basics: {name: Vinh Nguyen Phuc}\nsummary:\n  - Managed 9000+ computers.\n";
        assert_eq!(check(cv).len(), 1);
    }

    #[test]
    fn invented_project_school_cert_language_are_rejected() {
        let cv = "
basics: {name: Vinh Nguyen Phuc}
projects: [{name: Made Up Project}]
education: [{institution: MIT, degree: PhD}]
certifications: [{name: AWS Solutions Architect}]
languages: [{language: French, fluency: Native}]
";
        let paths: Vec<String> = check(cv).into_iter().map(|v| v.path).collect();
        for p in ["projects[0].name", "education[0].institution", "certifications[0].name", "languages[0].language"] {
            assert!(paths.contains(&p.to_string()), "{p} missing from {paths:?}");
        }
        let v = check(&EXAMPLE.replace("Basic working proficiency", "Fluent"));
        assert!(v.iter().any(|v| v.path == "languages[0].fluency"), "{}", messages(&v));
    }

    #[test]
    fn identity_must_match_the_profile() {
        let v = check(&EXAMPLE.replace("npvinh0507@gmail.com", "someone@else.com"));
        assert!(v.iter().any(|v| v.path == "basics.email"));
        let v = check(&EXAMPLE.replace("name: Vinh Nguyen Phuc", "name: Someone Else"));
        assert!(v.iter().any(|v| v.path == "basics.name"));
        let v = check(&EXAMPLE.replace("github.com/phucvinh57", "github.com/other"));
        assert!(v.iter().any(|v| v.path == "basics.github"));
        let v = check(&EXAMPLE.replace("\"0373395726\"", "\"0912345678\""));
        assert!(v.iter().any(|v| v.path == "basics.phone"));
        // a scheme or trailing slash on a link is not a change
        let v = check(&EXAMPLE.replace("github.com/phucvinh57", "https://www.github.com/phucvinh57/"));
        assert!(v.is_empty(), "{}", messages(&v));
    }

    #[test]
    fn skills_must_come_from_the_profile_with_synonyms() {
        let cv = "basics: {name: Vinh Nguyen Phuc}\nskills:\n  - {category: X, items: [Kubernetes, 'Postgres', Go, Rust, Docker, Cobol, Haskell]}\n";
        let v = check(cv);
        let bad: Vec<&str> = v.iter().map(|v| v.message.as_str()).collect();
        assert_eq!(v.len(), 2, "{bad:?}");
        assert!(bad.iter().any(|m| m.contains("Cobol")) && bad.iter().any(|m| m.contains("Haskell")));
        // synonyms both ways: the profile says K8s / PostgreSQL / TypeScript
        assert!(check("basics: {name: Vinh Nguyen Phuc}\nskills:\n  - {category: X, items: [TS, k8s, postgres, JS, Kubernetes]}\n")
            .is_empty());
        // short terms need a word boundary: `C` is not "Cloud"; `C++` is in the profile
        let v = check("basics: {name: Vinh Nguyen Phuc}\nskills:\n  - {category: X, items: [R, C++]}\n");
        assert_eq!(v.len(), 1);
        assert!(v[0].message.contains("`R`"));
    }

    #[test]
    fn unknown_location_is_rejected() {
        let v = check(&EXAMPLE.replace("location: Ho Chi Minh City, Vietnam", "location: Berlin, Germany"));
        assert!(v.iter().any(|v| v.path == "basics.location"), "{}", messages(&v));
    }

    #[test]
    fn lenient_parsing_accepts_numbers_and_missing_keys() {
        let cv = Cv::parse("basics: {name: Vinh Nguyen Phuc, phone: 123}\nexperience:\n  - {company: X, start: 2024, bullets: 'one'}\nunknown_key: 1\n").unwrap();
        assert_eq!(cv.experience[0].start, "2024");
        assert_eq!(cv.experience[0].bullets, ["one"]);
        assert_eq!(cv.basics.phone, "123");
        assert_eq!(Cv::parse("").unwrap(), Cv::default());
        assert!(Cv::parse("basics: [unclosed").is_err());
    }
}
