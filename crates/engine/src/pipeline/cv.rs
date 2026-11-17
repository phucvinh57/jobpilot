//! Typed mirror of the profile / `cv.yaml` schema.
//!
//! Every field is lenient (`#[serde(default)]`, scalars may be written as numbers, unknown keys are
//! ignored) so any CV the renderer accepts also parses here. The same types read the stored
//! profile and a tailored CV, which is what the fact guard compares.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use serde_yaml::Value;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cv {
    pub basics: Basics,
    /// Render order of the sections.
    pub sections: Vec<String>,
    /// Section title overrides.
    pub titles: BTreeMap<String, String>,
    /// Style overrides (`accent-color`, `font`, `font-size`, `paper`).
    pub style: BTreeMap<String, Value>,
    #[serde(deserialize_with = "de_strings")]
    pub summary: Vec<String>,
    pub skills: Vec<SkillGroup>,
    pub experience: Vec<Experience>,
    pub projects: Vec<Project>,
    pub education: Vec<Education>,
    pub certifications: Vec<Certification>,
    pub languages: Vec<Language>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Basics {
    #[serde(deserialize_with = "de_string")]
    pub name: String,
    #[serde(deserialize_with = "de_string")]
    pub headline: String,
    #[serde(deserialize_with = "de_string")]
    pub email: String,
    #[serde(deserialize_with = "de_string")]
    pub phone: String,
    #[serde(deserialize_with = "de_string")]
    pub location: String,
    #[serde(deserialize_with = "de_string")]
    pub github: String,
    #[serde(deserialize_with = "de_string")]
    pub linkedin: String,
    #[serde(deserialize_with = "de_string")]
    pub website: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillGroup {
    #[serde(deserialize_with = "de_string")]
    pub category: String,
    #[serde(deserialize_with = "de_strings")]
    pub items: Vec<String>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Experience {
    #[serde(deserialize_with = "de_string")]
    pub company: String,
    #[serde(deserialize_with = "de_string")]
    pub url: String,
    #[serde(deserialize_with = "de_string")]
    pub position: String,
    #[serde(deserialize_with = "de_string")]
    pub location: String,
    #[serde(deserialize_with = "de_string")]
    pub start: String,
    #[serde(deserialize_with = "de_string")]
    pub end: String,
    #[serde(deserialize_with = "de_strings")]
    pub bullets: Vec<String>,
    pub groups: Vec<Group>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Group {
    #[serde(deserialize_with = "de_string")]
    pub title: String,
    #[serde(deserialize_with = "de_strings")]
    pub bullets: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    #[serde(deserialize_with = "de_string")]
    pub name: String,
    #[serde(deserialize_with = "de_string")]
    pub url: String,
    #[serde(deserialize_with = "de_string")]
    pub role: String,
    #[serde(deserialize_with = "de_string")]
    pub start: String,
    #[serde(deserialize_with = "de_string")]
    pub end: String,
    #[serde(deserialize_with = "de_string")]
    pub intro: String,
    #[serde(deserialize_with = "de_strings")]
    pub bullets: Vec<String>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Education {
    #[serde(deserialize_with = "de_string")]
    pub institution: String,
    #[serde(deserialize_with = "de_string")]
    pub degree: String,
    #[serde(deserialize_with = "de_string")]
    pub location: String,
    #[serde(deserialize_with = "de_string")]
    pub gpa: String,
    #[serde(deserialize_with = "de_string")]
    pub start: String,
    #[serde(deserialize_with = "de_string")]
    pub end: String,
    #[serde(deserialize_with = "de_strings")]
    pub bullets: Vec<String>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Certification {
    #[serde(deserialize_with = "de_string")]
    pub name: String,
    #[serde(deserialize_with = "de_string")]
    pub issuer: String,
    #[serde(deserialize_with = "de_string")]
    pub url: String,
    #[serde(deserialize_with = "de_string")]
    pub date: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Language {
    #[serde(deserialize_with = "de_string")]
    pub language: String,
    #[serde(deserialize_with = "de_string")]
    pub fluency: String,
    pub hidden: bool,
}

impl Cv {
    /// Parse a CV or profile. An empty document is an empty CV; syntax and type errors carry their
    /// line/column so the user (or the model) can fix them.
    pub fn parse(yaml: &str) -> Result<Cv> {
        if yaml.trim().is_empty() {
            return Ok(Cv::default());
        }
        serde_yaml::from_str(yaml).context("invalid CV YAML")
    }

    /// Every free-text value a reader sees (headline, summary, skills, bullets, intros, group
    /// titles, roles, degrees, fluency) with a path for error messages. Used for the number check.
    pub fn prose(&self) -> Vec<(String, &str)> {
        let mut out: Vec<(String, &str)> = vec![("basics.headline".into(), &self.basics.headline)];
        for (i, s) in self.summary.iter().enumerate() {
            out.push((format!("summary[{i}]"), s));
        }
        for (i, g) in self.skills.iter().enumerate() {
            out.push((format!("skills[{i}].category"), &g.category));
            for (j, s) in g.items.iter().enumerate() {
                out.push((format!("skills[{i}].items[{j}]"), s));
            }
        }
        for (i, e) in self.experience.iter().enumerate() {
            out.push((format!("experience[{i}].position"), &e.position));
            for (j, b) in e.bullets.iter().enumerate() {
                out.push((format!("experience[{i}].bullets[{j}]"), b));
            }
            for (j, g) in e.groups.iter().enumerate() {
                out.push((format!("experience[{i}].groups[{j}].title"), &g.title));
                for (k, b) in g.bullets.iter().enumerate() {
                    out.push((format!("experience[{i}].groups[{j}].bullets[{k}]"), b));
                }
            }
        }
        for (i, p) in self.projects.iter().enumerate() {
            out.push((format!("projects[{i}].role"), &p.role));
            out.push((format!("projects[{i}].intro"), &p.intro));
            for (j, b) in p.bullets.iter().enumerate() {
                out.push((format!("projects[{i}].bullets[{j}]"), b));
            }
        }
        for (i, e) in self.education.iter().enumerate() {
            out.push((format!("education[{i}].degree"), &e.degree));
            out.push((format!("education[{i}].gpa"), &e.gpa));
            for (j, b) in e.bullets.iter().enumerate() {
                out.push((format!("education[{i}].bullets[{j}]"), b));
            }
        }
        for (i, l) in self.languages.iter().enumerate() {
            out.push((format!("languages[{i}].fluency"), &l.fluency));
        }
        out.retain(|(_, s)| !s.is_empty());
        out
    }

    /// Every string of the CV, lower-cased and joined: "somewhere in the profile".
    pub fn corpus(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let b = &self.basics;
        parts.extend(
            [&b.name, &b.headline, &b.email, &b.phone, &b.location, &b.github, &b.linkedin, &b.website]
                .map(String::as_str),
        );
        parts.extend(self.summary.iter().map(String::as_str));
        for g in &self.skills {
            parts.push(&g.category);
            parts.extend(g.items.iter().map(String::as_str));
        }
        for e in &self.experience {
            parts.extend([&e.company, &e.url, &e.position, &e.location, &e.start, &e.end].map(String::as_str));
            parts.extend(e.bullets.iter().map(String::as_str));
            for g in &e.groups {
                parts.push(&g.title);
                parts.extend(g.bullets.iter().map(String::as_str));
            }
        }
        for p in &self.projects {
            parts.extend([&p.name, &p.url, &p.role, &p.start, &p.end, &p.intro].map(String::as_str));
            parts.extend(p.bullets.iter().map(String::as_str));
        }
        for e in &self.education {
            parts.extend([&e.institution, &e.degree, &e.location, &e.gpa, &e.start, &e.end].map(String::as_str));
            parts.extend(e.bullets.iter().map(String::as_str));
        }
        for c in &self.certifications {
            parts.extend([&c.name, &c.issuer, &c.url, &c.date].map(String::as_str));
        }
        for l in &self.languages {
            parts.extend([&l.language, &l.fluency].map(String::as_str));
        }
        parts.retain(|s| !s.is_empty());
        parts.join("\n").to_lowercase()
    }
}

/// A scalar (string, number, bool) read as text; null reads as `""`; lists/mappings are rejected.
fn scalar(v: Value) -> Option<String> {
    match v {
        Value::Null => Some(String::new()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s),
        Value::Tagged(t) => scalar(t.value),
        Value::Sequence(_) | Value::Mapping(_) => None,
    }
}

fn de_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = Value::deserialize(d)?;
    scalar(v).ok_or_else(|| serde::de::Error::custom("expected text, found a list or mapping"))
}

fn de_strings<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(Vec::new()),
        Value::Sequence(items) => items
            .into_iter()
            .map(|v| scalar(v).ok_or_else(|| serde::de::Error::custom("expected text items, found a list or mapping")))
            .collect(),
        other => scalar(other).map(|s| vec![s]).ok_or_else(|| serde::de::Error::custom("expected a list of text")),
    }
}
