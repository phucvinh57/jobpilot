//! The company brief (`pipeline::Insight`) as the web app reads it. Discovery shows it under a
//! lead and Applications under the posting, so the read model is part of the shared kernel.

use jobpilot_core::pipeline::insight::{CompanyFacts, Interview};
use jobpilot_core::pipeline::Insight;
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsightDto {
    pub domain: String,
    pub business_summary: String,
    /// The company facts on one line (`Series B · 80 people · HQ Berlin`); `None` when none was found.
    pub facts: Option<String>,
    pub required_skills: Vec<String>,
    pub preferred_skills: Vec<String>,
    /// The interview stages in order; empty when the process was not found.
    pub interview_steps: Vec<String>,
    /// Notes on the interview process; `None` when it was not found or says nothing.
    pub interview_notes: Option<String>,
    /// http(s) pages the brief comes from.
    pub sources: Vec<String>,
}

impl From<&Insight> for InsightDto {
    fn from(insight: &Insight) -> Self {
        let notes = insight.interview.notes.trim();
        Self {
            domain: insight.domain.trim().to_string(),
            business_summary: insight.business_summary.trim().to_string(),
            facts: facts_line(&insight.company),
            required_skills: insight.skills.required.clone(),
            preferred_skills: insight.skills.preferred.clone(),
            interview_steps: interview_steps(&insight.interview),
            interview_notes: (insight.interview.found && !notes.is_empty())
                .then(|| notes.to_string()),
            sources: insight
                .sources
                .iter()
                .filter(|u| is_web_link(u))
                .cloned()
                .collect(),
        }
    }
}

/// An `http(s)` address: the only kind of link the app opens.
pub fn is_web_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// The company facts the texts gave, as one line; `None` when they gave none.
pub fn facts_line(c: &CompanyFacts) -> Option<String> {
    let part = |label: &str, value: &str| {
        let value = value.trim();
        (!value.is_empty()).then(|| {
            if label.is_empty() {
                value.to_string()
            } else {
                format!("{label} {value}")
            }
        })
    };
    let parts: Vec<String> = [
        part("", &c.stage),
        part("", &c.size),
        part("Funding", &c.funding),
        part("HQ", &c.hq),
        part("Founded", &c.founded),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// The interview stages in order; empty when the process was not found.
pub fn interview_steps(i: &Interview) -> Vec<String> {
    if !i.found {
        return Vec::new();
    }
    i.stages
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use jobpilot_core::pipeline::insight::{CompanyFacts, Interview};
    use jobpilot_core::pipeline::Insight;

    use super::{facts_line, interview_steps, InsightDto};

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn facts_read_as_one_line() {
        let facts = CompanyFacts {
            stage: "Series B".into(),
            size: "80 people".into(),
            hq: "Berlin".into(),
            ..CompanyFacts::default()
        };
        assert_eq!(
            facts_line(&facts).as_deref(),
            Some("Series B \u{b7} 80 people \u{b7} HQ Berlin")
        );
        assert_eq!(facts_line(&CompanyFacts::default()), None);
        let blank = CompanyFacts {
            size: "  ".into(),
            ..CompanyFacts::default()
        };
        assert_eq!(facts_line(&blank), None);
    }

    #[test]
    fn interview_steps_need_a_found_process() {
        let found = Interview {
            stages: names(&["Recruiter call", " ", "Take-home"]),
            notes: String::new(),
            found: true,
        };
        assert_eq!(
            interview_steps(&found),
            names(&["Recruiter call", "Take-home"])
        );
        let not_found = Interview {
            stages: names(&["Stale"]),
            notes: String::new(),
            found: false,
        };
        assert!(interview_steps(&not_found).is_empty());
    }

    #[test]
    fn the_read_model_keeps_only_web_links_and_found_notes() {
        let insight = Insight {
            sources: names(&[
                "https://a.example.com",
                "file:///etc/passwd",
                "javascript:alert(1)",
            ]),
            interview: Interview {
                stages: Vec::new(),
                notes: "stale".into(),
                found: false,
            },
            ..Insight::default()
        };
        let dto = InsightDto::from(&insight);
        assert_eq!(dto.sources, names(&["https://a.example.com"]));
        assert_eq!(dto.interview_notes, None);
    }
}
