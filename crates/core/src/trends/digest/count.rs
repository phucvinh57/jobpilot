//! The `count` node: how often each skill is asked for, per month, counted in code.
//!
//! Documents are the "Who is hiring" postings among the collected items and the leads discovery
//! found. A document counts once for a skill however often it names it. The vocabulary is fixed
//! (`BUILT_IN`) plus the skills in the user's preferences, so the same documents always give the
//! same numbers; the model has no part in it.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Utc};
use regex::Regex;

use super::{read_text, DigestEnv, SkillCount, SkillCounts};
use crate::prefs::Preferences;
use crate::store::{Lead, LeadState};
use crate::trends::collect::TrendRun;
use crate::trends::period::Period;

/// Months in the window, the period's month last.
const MONTHS: usize = 6;
/// Skills kept in the result.
const MAX_SKILLS: usize = 30;
/// Characters of a lead's posting that are read.
const MAX_LEAD_CHARS: usize = 20_000;
/// Permalink of a Hacker News item: where hiring postings live.
const HN_ITEM_PREFIX: &str = "https://news.ycombinator.com/item?id=";

/// `(name, aliases)`. An alias is matched as whole words, ignoring case; one that starts with `=`
/// must match exactly as written (`=Go`: the language, not the verb).
const BUILT_IN: &[(&str, &[&str])] = &[
    ("Rust", &["rust"]),
    ("Go", &["=Go", "golang"]),
    ("Python", &["python"]),
    ("Java", &["java"]),
    ("Kotlin", &["kotlin"]),
    ("Scala", &["scala"]),
    ("TypeScript", &["typescript"]),
    ("JavaScript", &["javascript"]),
    ("C++", &["c++"]),
    ("C#", &["c#"]),
    ("Swift", &["=Swift"]),
    ("Ruby", &["ruby"]),
    ("PHP", &["php"]),
    ("Elixir", &["elixir"]),
    ("Erlang", &["erlang"]),
    ("Haskell", &["haskell"]),
    ("Clojure", &["clojure"]),
    ("OCaml", &["ocaml"]),
    ("Zig", &["=Zig"]),
    ("Dart", &["=Dart"]),
    ("SQL", &["sql"]),
    ("React", &["react", "react.js", "reactjs"]),
    ("React Native", &["react native"]),
    ("Vue", &["vue", "vue.js", "vuejs"]),
    ("Angular", &["angular"]),
    ("Svelte", &["svelte"]),
    ("Next.js", &["next.js", "nextjs"]),
    ("Node.js", &["node.js", "nodejs"]),
    ("Django", &["django"]),
    ("Flask", &["flask"]),
    ("FastAPI", &["fastapi"]),
    ("Spring", &["=Spring"]),
    ("Rails", &["rails"]),
    ("Laravel", &["laravel"]),
    (".NET", &[".net", "asp.net", "dotnet"]),
    ("GraphQL", &["graphql"]),
    ("gRPC", &["grpc"]),
    ("Flutter", &["flutter"]),
    ("Android", &["android"]),
    ("iOS", &["ios"]),
    ("PostgreSQL", &["postgres", "postgresql"]),
    ("MySQL", &["mysql"]),
    ("MongoDB", &["mongodb", "mongo"]),
    ("Redis", &["redis"]),
    ("Kafka", &["kafka"]),
    ("Spark", &["=Spark", "pyspark"]),
    ("Flink", &["flink"]),
    ("Snowflake", &["snowflake"]),
    ("BigQuery", &["bigquery"]),
    ("Databricks", &["databricks"]),
    ("dbt", &["dbt"]),
    ("Airflow", &["airflow"]),
    ("Elasticsearch", &["elasticsearch", "opensearch"]),
    ("ClickHouse", &["clickhouse"]),
    ("Cassandra", &["cassandra"]),
    ("DynamoDB", &["dynamodb"]),
    ("SQLite", &["sqlite"]),
    ("AWS", &["aws"]),
    ("GCP", &["gcp", "google cloud"]),
    ("Azure", &["azure"]),
    ("Kubernetes", &["kubernetes", "k8s"]),
    ("Docker", &["docker"]),
    ("Terraform", &["terraform"]),
    ("Ansible", &["ansible"]),
    ("Helm", &["=Helm"]),
    ("Linux", &["linux"]),
    ("Prometheus", &["prometheus"]),
    ("Grafana", &["grafana"]),
    ("GitHub Actions", &["github actions"]),
    ("Nix", &["=Nix", "nixos"]),
    ("Machine Learning", &["machine learning"]),
    ("PyTorch", &["pytorch"]),
    ("TensorFlow", &["tensorflow"]),
    ("LLM", &["llm", "llms"]),
    ("Generative AI", &["generative ai", "genai"]),
    ("RAG", &["=RAG"]),
    ("LangChain", &["langchain"]),
    ("Computer Vision", &["computer vision"]),
    ("NLP", &["=NLP", "natural language processing"]),
    ("WebAssembly", &["wasm", "webassembly"]),
    ("Solidity", &["solidity"]),
    ("Blockchain", &["blockchain"]),
    ("Microservices", &["microservice", "microservices"]),
    ("Distributed Systems", &["distributed systems"]),
    ("Observability", &["observability"]),
];

/// Words of a text: runs of letters and digits that may contain `+ # . -` (`c++`, `node.js`,
/// `front-end`), or `.NET`. Trailing dots and dashes are cut by [`tokens`].
fn word_pattern() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| Regex::new(r"(?i)\.net\b|[a-z0-9][a-z0-9+#.\-]*").expect("static regex"))
}

/// `(as written, lower case)` per word.
fn tokens(text: &str) -> Vec<(&str, String)> {
    word_pattern()
        .find_iter(text)
        .map(|m| {
            let word = m.as_str().trim_end_matches(['.', '-']);
            (word, word.to_lowercase())
        })
        .filter(|(word, _)| !word.is_empty())
        .collect()
}

struct Alias {
    skill: usize,
    /// Lower-case words after the first.
    rest: Vec<String>,
    /// The first word exactly as written, for aliases that must match case.
    exact: Option<String>,
}

/// The skills that are counted and the words that stand for them.
pub struct Vocabulary {
    names: Vec<String>,
    by_first_word: HashMap<String, Vec<Alias>>,
}

impl Vocabulary {
    /// The built-in skills plus `extra` (the user's skills; one that is already built in adds
    /// nothing).
    pub fn new(extra: &[String]) -> Self {
        let mut vocab = Vocabulary { names: Vec::new(), by_first_word: HashMap::new() };
        for (name, aliases) in BUILT_IN {
            vocab.add(name, aliases);
        }
        for skill in extra {
            let skill = skill.split_whitespace().collect::<Vec<_>>().join(" ");
            if !skill.is_empty() {
                vocab.add(&skill, &[&skill.to_lowercase()]);
            }
        }
        vocab
    }

    fn add(&mut self, name: &str, aliases: &[&str]) {
        if self.names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return;
        }
        let skill = self.names.len();
        for alias in aliases {
            let (alias, exact) = match alias.strip_prefix('=') {
                Some(plain) => (plain, true),
                None => (*alias, false),
            };
            let words = tokens(alias);
            let Some(((written, first), rest)) = words.split_first() else { continue };
            self.by_first_word.entry(first.clone()).or_default().push(Alias {
                skill,
                rest: rest.iter().map(|(_, lower)| lower.clone()).collect(),
                exact: exact.then(|| (*written).to_string()),
            });
        }
        self.names.push(name.to_string());
    }

    /// The skills a text names, by index into `names`.
    fn skills_in(&self, text: &str) -> BTreeSet<usize> {
        let words = tokens(text);
        let mut found = BTreeSet::new();
        for (i, (written, lower)) in words.iter().enumerate() {
            let Some(candidates) = self.by_first_word.get(lower) else { continue };
            for alias in candidates {
                let exact_ok = alias.exact.as_deref().is_none_or(|e| e == *written);
                let rest_ok = alias.rest.iter().enumerate().all(|(k, w)| words.get(i + 1 + k).is_some_and(|(_, l)| l == w));
                if exact_ok && rest_ok {
                    found.insert(alias.skill);
                }
            }
        }
        found
    }
}

/// One counted document: the month it belongs to (`2026-11`) and its text.
pub(super) struct Doc {
    pub month: String,
    pub text: String,
}

/// `n` months ending with the month of the period's last day, oldest first (`2026-06` ...).
pub fn month_window(period: &Period, n: usize) -> Vec<String> {
    let last = (period.end() - Duration::days(1)).date_naive();
    let (mut year, mut month) = (last.year(), last.month());
    let mut months = Vec::with_capacity(n);
    for _ in 0..n {
        months.push(format!("{year:04}-{month:02}"));
        if month == 1 {
            (year, month) = (year - 1, 12);
        } else {
            month -= 1;
        }
    }
    months.reverse();
    months
}

/// Skill frequency over `docs` (those outside `months` are ignored): per skill and month the
/// number of documents that name it. Most mentioned first, ties by name, at most [`MAX_SKILLS`].
pub(super) fn count_skills(docs: &[Doc], vocab: &Vocabulary, months: &[String]) -> SkillCounts {
    let column: HashMap<&str, usize> = months.iter().enumerate().map(|(i, m)| (m.as_str(), i)).collect();
    let mut documents = vec![0u32; months.len()];
    let mut table: Vec<Vec<u32>> = vec![vec![0; months.len()]; vocab.names.len()];
    for doc in docs {
        let Some(&at) = column.get(doc.month.as_str()) else { continue };
        documents[at] += 1;
        for skill in vocab.skills_in(&doc.text) {
            table[skill][at] += 1;
        }
    }
    let mut skills: Vec<SkillCount> = vocab
        .names
        .iter()
        .zip(table)
        .map(|(name, by_month)| SkillCount { skill: name.clone(), total: by_month.iter().sum(), by_month })
        .filter(|s| s.total > 0)
        .collect();
    skills.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.skill.cmp(&b.skill)));
    skills.truncate(MAX_SKILLS);
    SkillCounts { months: months.to_vec(), documents, skills }
}

/// A "Who is hiring" posting as the Hacker News adapter writes it: its text starts with
/// `# Hacker News, Ask HN: Who is hiring? (...)`.
pub fn is_hiring_posting(text: &str) -> bool {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_lowercase();
    first.starts_with("# hacker news,") && first.contains("who is hiring")
}

fn month_of(time: DateTime<Utc>) -> String {
    time.format("%Y-%m").to_string()
}

pub(super) async fn count(env: Arc<DigestEnv>, mut s: TrendRun) -> Result<TrendRun> {
    let months = month_window(&s.period, MONTHS);
    let in_window = |month: &str| months.iter().any(|m| m == month);
    let mut docs = Vec::new();

    for item in env.store.list_trend_items(None)? {
        let month = month_of(item.published_at.unwrap_or(item.fetched_at));
        if item.url.starts_with(HN_ITEM_PREFIX) && in_window(&month) {
            let text = read_text(&item).await;
            if is_hiring_posting(&text) {
                docs.push(Doc { month, text });
            }
        }
    }
    let mut leads = 0usize;
    for state in [LeadState::New, LeadState::Promoted, LeadState::Dismissed] {
        for lead in env.store.list_leads(state)? {
            let month = month_of(lead.found_at);
            if in_window(&month) {
                leads += 1;
                docs.push(Doc { month, text: lead_text(&lead).await });
            }
        }
    }

    let prefs = env.store.preferences().unwrap_or_else(|_| {
        tracing::warn!("the saved preferences could not be read; only the built-in skills are counted");
        Preferences::default()
    });
    let extra: Vec<String> = prefs.must_skills.iter().chain(&prefs.nice_skills).chain(&prefs.avoid_skills).cloned().collect();
    let counts = count_skills(&docs, &Vocabulary::new(&extra), &months);
    tracing::info!(documents = docs.len(), leads, skills = counts.skills.len(), "skill frequency counted");
    s.digest.counts = Some(counts);
    Ok(s)
}

/// A lead's role and posting text.
async fn lead_text(lead: &Lead) -> String {
    let body = match &lead.jd_path {
        Some(path) => tokio::fs::read_to_string(path).await.unwrap_or_default(),
        None => String::new(),
    };
    format!("{}\n{}", lead.role, crate::trends::text::cap(&body, MAX_LEAD_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(month: &str, text: &str) -> Doc {
        Doc { month: month.into(), text: text.into() }
    }

    fn months() -> Vec<String> {
        ["2026-09", "2026-10", "2026-11"].map(String::from).to_vec()
    }

    fn total(counts: &SkillCounts, skill: &str) -> Option<Vec<u32>> {
        counts.skills.iter().find(|s| s.skill == skill).map(|s| s.by_month.clone())
    }

    #[test]
    fn skills_are_counted_once_per_document_and_month() {
        let docs = [
            doc("2026-09", "Acme | Rust, Rust and more Rust | We use Kafka"),
            doc("2026-10", "Globex | Senior Rust engineer | Go, Kubernetes (K8s) and Postgres"),
            doc("2026-10", "Initech | PostgreSQL DBA | AWS, docker"),
            doc("2026-11", "Hooli | We go to market fast. Rust! Machine Learning, node.js and C++/C#."),
            doc("2026-12", "Outside the window: Rust Rust"),
        ];
        let counts = count_skills(&docs, &Vocabulary::new(&[]), &months());

        assert_eq!(counts.months, months());
        assert_eq!(counts.documents, [1, 2, 1], "the December document is ignored");
        assert_eq!(total(&counts, "Rust"), Some(vec![1, 1, 1]), "once per document");
        assert_eq!(total(&counts, "Kafka"), Some(vec![1, 0, 0]));
        assert_eq!(total(&counts, "Go"), Some(vec![0, 1, 0]), "the language, not the verb in 'we go to market'");
        assert_eq!(total(&counts, "Kubernetes"), Some(vec![0, 1, 0]), "K8s counts as Kubernetes, once");
        assert_eq!(total(&counts, "PostgreSQL"), Some(vec![0, 2, 0]), "Postgres and PostgreSQL");
        assert_eq!(total(&counts, "Docker"), Some(vec![0, 1, 0]), "ignoring case");
        assert_eq!(total(&counts, "Machine Learning"), Some(vec![0, 0, 1]), "two words");
        assert_eq!(total(&counts, "Node.js"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "C++"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "C#"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "Java"), None, "absent skills are not listed");
        // Most mentioned first, ties by name.
        assert_eq!(counts.skills[0].skill, "Rust");
        assert_eq!(counts.skills[1].skill, "PostgreSQL");
        let tied: Vec<&str> = counts.skills.iter().filter(|s| s.total == 1).map(|s| s.skill.as_str()).collect();
        let mut sorted = tied.clone();
        sorted.sort_unstable();
        assert_eq!(tied, sorted);
    }

    #[test]
    fn counting_is_deterministic_and_does_not_confuse_similar_words() {
        let docs = [doc("2026-11", "JavaScript and TypeScript, not Java. Trust us. Mongo, MongoDB. Spark of joy; Apache Spark.")];
        let a = count_skills(&docs, &Vocabulary::new(&[]), &months());
        let b = count_skills(&docs, &Vocabulary::new(&[]), &months());
        assert_eq!(a, b);
        assert_eq!(total(&a, "Rust"), None, "'Trust' is not Rust");
        assert_eq!(total(&a, "Java"), Some(vec![0, 0, 1]), "JavaScript is not Java");
        assert_eq!(total(&a, "JavaScript"), Some(vec![0, 0, 1]));
        assert_eq!(total(&a, "MongoDB"), Some(vec![0, 0, 1]));
        assert_eq!(total(&a, "Spark"), Some(vec![0, 0, 1]));
    }

    #[test]
    fn the_users_own_skills_are_counted_too() {
        let docs = [doc("2026-11", "We need Gleam and Crystal developers, and rust too")];
        let vocab = Vocabulary::new(&["Gleam".into(), "crystal".into(), "rust".into(), "  ".into()]);
        let counts = count_skills(&docs, &vocab, &months());
        assert_eq!(total(&counts, "Gleam"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "crystal"), Some(vec![0, 0, 1]), "named as the user wrote it");
        assert_eq!(counts.skills.iter().filter(|s| s.skill.eq_ignore_ascii_case("rust")).count(), 1, "no duplicate of a built-in");
    }

    #[test]
    fn the_window_ends_with_the_month_of_the_period() {
        let w46 = Period::parse("2026-W46").unwrap();
        assert_eq!(month_window(&w46, 3), ["2026-09", "2026-10", "2026-11"]);
        let w1 = Period::parse("2026-W01").unwrap();
        assert_eq!(month_window(&w1, 3), ["2025-11", "2025-12", "2026-01"], "W01 ends on 2026-01-04");
        let q4 = Period::parse("2026-Q4").unwrap();
        assert_eq!(month_window(&q4, 6), ["2026-07", "2026-08", "2026-09", "2026-10", "2026-11", "2026-12"]);
    }

    #[test]
    fn hiring_postings_are_recognised_by_their_header() {
        assert!(is_hiring_posting("# Hacker News, Ask HN: Who is hiring? (November 2026)\n\nAcme | Rust"));
        assert!(!is_hiring_posting("# Rust 2.0 released\n\n450 points"));
        assert!(!is_hiring_posting("# Hacker News, Ask HN: Who wants to be hired?\n\n..."));
        assert!(!is_hiring_posting(""));
    }
}
