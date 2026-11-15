//! Trace check: a deterministic check that every fact the reader extracted is in the user's
//! documents. It is the fact guard (`pipeline::guard`) one step earlier: the guard keeps a CV to
//! the profile; this keeps the profile to the documents.
//!
//! Names, titles, employers, schools, dates, contact details, links and skills must appear in the
//! text (case and spacing aside); every number in prose must appear in the text. Prose itself
//! may be reworded. What fails is not kept: an entry without its employer, school or name is
//! dropped, a field is cleared, a line with an unknown number is dropped. Each is reported in
//! plain words so the user can add the fact to a document.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;

use crate::engine::pipeline::cv::Cv;
use crate::engine::pipeline::guard::{contains_word, norm, norm_phone, norm_url, number_tokens, term_in_corpus};

/// What the user's documents say, normalised for lookups.
struct Corpus {
    /// Lower case, whitespace collapsed.
    text: String,
    /// Every number token of the text.
    numbers: BTreeSet<String>,
    /// Every phone-like run of the text, digits and `+` only.
    phones: Vec<String>,
}

impl Corpus {
    fn new(text: &str) -> Self {
        static PHONE: OnceLock<Regex> = OnceLock::new();
        let phone = PHONE.get_or_init(|| Regex::new(r"\+?\d[\d\s().\-/]{5,}\d").expect("static regex"));
        Self {
            text: norm(text),
            numbers: number_tokens(text).into_iter().collect(),
            phones: phone.find_iter(text).map(|m| norm_phone(m.as_str())).collect(),
        }
    }

    /// `value` (or nothing) is in the text as whole words: `MIT` is not found in "submitted".
    fn has(&self, value: &str) -> bool {
        let v = norm(value);
        v.is_empty() || contains_word(&self.text, &v)
    }

    fn has_url(&self, value: &str) -> bool {
        let v = norm_url(value);
        v.is_empty() || self.text.contains(&v)
    }

    fn has_phone(&self, value: &str) -> bool {
        let v = norm_phone(value);
        v.is_empty() || self.phones.iter().any(|p| p.trim_start_matches('+').contains(v.trim_start_matches('+')))
    }

    /// The first number of `text` that is not in the documents.
    fn unknown_number(&self, text: &str) -> Option<String> {
        number_tokens(text).into_iter().find(|n| !self.numbers.contains(n))
    }
}

/// Keep only what `documents` support. Returns the kept profile and what was left out.
pub fn trace(mut cv: Cv, documents: &str) -> (Cv, Vec<String>) {
    let corpus = Corpus::new(documents);
    let mut left_out = Vec::new();
    let mut out = |line: String| {
        if !left_out.contains(&line) {
            left_out.push(line);
        }
    };

    // ---- identity
    let b = &mut cv.basics;
    let traced = [
        corpus.has(&b.name),
        corpus.has(&b.email),
        corpus.has(&b.location),
        corpus.has_url(&b.github),
        corpus.has_url(&b.linkedin),
        corpus.has_url(&b.website),
    ];
    let fields = [
        ("Name", &mut b.name),
        ("Email", &mut b.email),
        ("Location", &mut b.location),
        ("GitHub", &mut b.github),
        ("LinkedIn", &mut b.linkedin),
        ("Website", &mut b.website),
    ];
    for ((label, field), ok) in fields.into_iter().zip(traced) {
        if !ok {
            out(format!("{label} “{field}”"));
            field.clear();
        }
    }
    if !corpus.has_phone(&b.phone) {
        out(format!("Phone “{}”", b.phone));
        b.phone.clear();
    }
    if let Some(n) = corpus.unknown_number(&b.headline) {
        out(format!("Headline “{}” (the number {n})", b.headline));
        b.headline.clear();
    }

    // ---- summary
    cv.summary.retain(|line| match corpus.unknown_number(line) {
        Some(n) => {
            out(format!("A summary line with the number {n}"));
            false
        }
        None => true,
    });

    // ---- skills
    for group in &mut cv.skills {
        group.items.retain(|item| {
            let ok = term_in_corpus(&corpus.text, item);
            if !ok {
                out(format!("Skill “{item}”"));
            }
            ok
        });
    }
    cv.skills.retain(|g| !g.items.is_empty());

    // ---- experience
    cv.experience.retain_mut(|e| {
        if e.company.trim().is_empty() || !corpus.has(&e.company) || !corpus.has(&e.position) {
            out(format!("Role “{}” at “{}”", e.position, e.company));
            return false;
        }
        let at = e.company.clone();
        clear_untraced(&corpus, &mut out, &at, [("dates", &mut e.start), ("dates", &mut e.end), ("location", &mut e.location)]);
        if !corpus.has_url(&e.url) {
            out(format!("Link of {at}: “{}”", e.url));
            e.url.clear();
        }
        keep_known_lines(&corpus, &mut out, &at, &mut e.bullets);
        for g in &mut e.groups {
            keep_known_lines(&corpus, &mut out, &at, &mut g.bullets);
        }
        e.groups.retain(|g| !g.bullets.is_empty());
        true
    });

    // ---- projects
    cv.projects.retain_mut(|p| {
        if p.name.trim().is_empty() || !corpus.has(&p.name) {
            out(format!("Project “{}”", p.name));
            return false;
        }
        let at = p.name.clone();
        clear_untraced(&corpus, &mut out, &at, [("role", &mut p.role), ("dates", &mut p.start), ("dates", &mut p.end)]);
        if !corpus.has_url(&p.url) {
            out(format!("Link of {at}: “{}”", p.url));
            p.url.clear();
        }
        if let Some(n) = corpus.unknown_number(&p.intro) {
            out(format!("A line about {at} with the number {n}"));
            p.intro.clear();
        }
        keep_known_lines(&corpus, &mut out, &at, &mut p.bullets);
        true
    });

    // ---- education
    cv.education.retain_mut(|e| {
        if e.institution.trim().is_empty() || !corpus.has(&e.institution) {
            out(format!("School “{}”", e.institution));
            return false;
        }
        let at = e.institution.clone();
        clear_untraced(
            &corpus,
            &mut out,
            &at,
            [("degree", &mut e.degree), ("location", &mut e.location), ("dates", &mut e.start), ("dates", &mut e.end)],
        );
        if !e.gpa.is_empty() && (!corpus.has(&e.gpa) || corpus.unknown_number(&e.gpa).is_some()) {
            out(format!("GPA of {at}: “{}”", e.gpa));
            e.gpa.clear();
        }
        keep_known_lines(&corpus, &mut out, &at, &mut e.bullets);
        true
    });

    // ---- certifications
    cv.certifications.retain_mut(|c| {
        if c.name.trim().is_empty() || !corpus.has(&c.name) {
            out(format!("Certification “{}”", c.name));
            return false;
        }
        let at = c.name.clone();
        clear_untraced(&corpus, &mut out, &at, [("issuer", &mut c.issuer), ("date", &mut c.date)]);
        if !corpus.has_url(&c.url) {
            c.url.clear();
        }
        true
    });

    // ---- languages
    cv.languages.retain_mut(|l| {
        if l.language.trim().is_empty() || !corpus.has(&l.language) {
            out(format!("Language “{}”", l.language));
            return false;
        }
        let at = l.language.clone();
        clear_untraced(&corpus, &mut out, &at, [("level", &mut l.fluency)]);
        true
    });

    (cv, left_out)
}

/// Clear each field the documents do not contain, reporting it as "`what` of `at`".
fn clear_untraced<const N: usize>(
    corpus: &Corpus,
    out: &mut impl FnMut(String),
    at: &str,
    fields: [(&str, &mut String); N],
) {
    for (what, field) in fields {
        if !corpus.has(field) {
            out(format!("The {what} of {at}: “{field}”"));
            field.clear();
        }
    }
}

/// Drop the lines that state a number the documents do not have.
fn keep_known_lines(corpus: &Corpus, out: &mut impl FnMut(String), at: &str, lines: &mut Vec<String>) {
    lines.retain(|line| match corpus.unknown_number(line) {
        Some(n) => {
            out(format!("A line about {at} with the number {n}"));
            false
        }
        None => true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "Ann Lee — ann@lee.dev — +84 912 345 678 — github.com/annlee
Backend engineer in Hanoi.
Acme Corp, Senior Engineer, Jan 2021 – Present: cut latency by 40% for 3 teams.
Skills: Rust, PostgreSQL, Kubernetes.
Hanoi University of Science, BSc Computer Science, 2015 - 2019.
English (fluent)";

    fn extracted() -> Cv {
        Cv::parse(
            r#"
basics: {name: Ann Lee, email: ann@lee.dev, phone: "+84912345678", github: "https://github.com/annlee/", location: Hanoi, headline: Backend engineer}
summary: ["Backend engineer with 10 years of experience", "Ships reliable services"]
skills:
  - {category: Languages, items: [Rust, Go]}
  - {category: Data, items: [Postgres, k8s]}
  - {category: Cloud, items: [AWS]}
experience:
  - {company: Acme Corp, position: Senior Engineer, start: Jan 2021, end: Present, location: Hanoi,
     bullets: ["Cut latency by 40% for 3 teams", "Cut costs by 25%"]}
  - {company: Globex, position: CTO, start: "2010", end: "2014"}
  - {company: Acme Corp, position: Staff Engineer}
education:
  - {institution: Hanoi University of Science, degree: BSc Computer Science, start: "2015", end: "2020", gpa: "3.9"}
  - {institution: MIT, degree: PhD}
languages:
  - {language: English, fluency: fluent}
  - {language: French, fluency: native}
certifications:
  - {name: CKA, issuer: CNCF}
"#,
        )
        .unwrap()
    }

    #[test]
    fn traced_facts_are_kept() {
        let (cv, _) = trace(extracted(), DOCUMENT);
        let b = &cv.basics;
        assert_eq!((b.name.as_str(), b.email.as_str(), b.phone.as_str()), ("Ann Lee", "ann@lee.dev", "+84912345678"));
        assert_eq!((b.github.as_str(), b.location.as_str(), b.headline.as_str()), ("https://github.com/annlee/", "Hanoi", "Backend engineer"));
        assert_eq!(cv.summary, ["Ships reliable services"]);
        let acme = &cv.experience[0];
        assert_eq!((acme.position.as_str(), acme.start.as_str(), acme.end.as_str()), ("Senior Engineer", "Jan 2021", "Present"));
        assert_eq!(acme.bullets, ["Cut latency by 40% for 3 teams"]);
        assert_eq!(cv.education[0].degree, "BSc Computer Science");
        assert_eq!(cv.education[0].start, "2015");
        assert_eq!(cv.languages[0].fluency, "fluent");
        // synonyms count: Postgres/PostgreSQL, k8s/Kubernetes
        assert_eq!(cv.skills.iter().flat_map(|g| g.items.clone()).collect::<Vec<_>>(), ["Rust", "Postgres", "k8s"]);
    }

    #[test]
    fn untraced_facts_are_left_out_and_reported() {
        let (cv, left_out) = trace(extracted(), DOCUMENT);
        assert_eq!(cv.experience.len(), 1, "an unknown employer or title drops the role");
        assert_eq!(cv.education.len(), 1);
        assert_eq!(cv.education[0].end, "", "an unknown date is cleared");
        assert_eq!(cv.education[0].gpa, "");
        assert_eq!(cv.languages.len(), 1);
        assert!(cv.certifications.is_empty());
        assert_eq!(cv.skills.len(), 2, "a group with no skill left is dropped");
        for expected in [
            "A summary line with the number 10",
            "Skill “Go”",
            "Skill “AWS”",
            "Role “CTO” at “Globex”",
            "Role “Staff Engineer” at “Acme Corp”",
            "A line about Acme Corp with the number 25",
            "School “MIT”",
            "The dates of Hanoi University of Science: “2020”",
            "GPA of Hanoi University of Science: “3.9”",
            "Language “French”",
            "Certification “CKA”",
        ] {
            assert!(left_out.iter().any(|l| l == expected), "missing {expected:?} in {left_out:#?}");
        }
    }

    #[test]
    fn a_short_name_must_be_a_whole_word() {
        let mut cv = extracted();
        cv.education[1].degree.clear();
        let (cv, _) = trace(cv, &format!("{DOCUMENT}\nI submitted 40 commits."));
        assert!(cv.education.iter().all(|e| e.institution != "MIT"), "MIT is not in “submitted”");
    }

    #[test]
    fn contact_details_not_in_the_documents_are_cleared() {
        let mut cv = extracted();
        cv.basics.phone = "+1 555 0100".into();
        cv.basics.email = "ann@other.dev".into();
        cv.basics.linkedin = "linkedin.com/in/ann".into();
        let (cv, left_out) = trace(cv, DOCUMENT);
        assert_eq!((cv.basics.phone.as_str(), cv.basics.email.as_str(), cv.basics.linkedin.as_str()), ("", "", ""));
        assert!(left_out.contains(&"Phone “+1 555 0100”".to_string()), "{left_out:?}");
    }
}
