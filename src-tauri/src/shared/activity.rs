//! The background jobs the sidebar lists: every graph run that is running in this process or
//! waiting for the user, with a title and one line about what it does now.
//!
//! Pure rules over what `core.graphs` reports; the read itself is `shell::shared_activity`.

use jobpilot_engine::discover::promote::PREPARE_GRAPH;
use jobpilot_engine::discover::suggest::SUGGEST_GRAPH;
use jobpilot_engine::discover::{parse_source_thread, DISCOVERY_GRAPH, SOURCE_GRAPH};
use jobpilot_engine::graph::WaitReason;
use jobpilot_engine::pipeline::insight::GRAPH as INSIGHT_GRAPH;
use jobpilot_engine::prefs::chat_graph::GRAPH as PREFS_CHAT_GRAPH;
use jobpilot_engine::trends::collect::GRAPH as TRENDS_GRAPH;
use jobpilot_engine::trends::digest::DIGEST_GRAPH as TRENDS_DIGEST_GRAPH;
use jobpilot_engine::trends::Period;
use serde::Serialize;

/// What a running job says before its first node is known, and for a node without a label.
const STARTING: &str = "Starting";
const WORKING: &str = "Working";

/// A graph thread as the engine reports it: the input of [`activity`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub thread: String,
    pub graph: String,
    /// The nodes it is at (several after a fan-out); empty before the first one is known.
    pub nodes: Vec<String>,
    /// `Some` for a thread paused for the user, `None` for one that runs.
    pub waiting: Option<WaitReason>,
    /// The name of the job source a `source_run` thread searches, when it can be read.
    pub source: Option<String>,
}

/// One line of the sidebar's "background jobs" list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDto {
    /// The graph thread: progress events of this thread belong to this line.
    pub id: String,
    pub title: String,
    /// What it does now, or what it needs from the user.
    pub status: String,
    /// `running` / `waiting`.
    pub state: &'static str,
    /// The screen that shows it: `jobs` / `trends` / `settings` (Job sources).
    pub screen: &'static str,
}

/// `Week of 9 November`, `November 2026`, `Q4 2026` or `2026`.
pub fn period_title(period: Period) -> String {
    match period {
        Period::Week { .. } => format!("Week of {}", period.start().format("%-d %B")),
        Period::Month { .. } => period.start().format("%B %Y").to_string(),
        Period::Quarter { year, quarter } => format!("Q{quarter} {year}"),
        Period::Year { year } => year.to_string(),
    }
}

/// What a node of `graph` does, in the words the screens use for the same step (the trends labels
/// are those of the web app's `src/lib/trends/jobs.ts`: change both together). `None` for a node
/// that says nothing of its own (a router, an unknown node).
fn node_label(graph: &str, node: &str) -> Option<&'static str> {
    let label = match (graph, node) {
        (DISCOVERY_GRAPH, "plan") => "Choosing the sources",
        (DISCOVERY_GRAPH, "sources") => "Searching the sources",
        (DISCOVERY_GRAPH, "summary") => "Summing up",

        (SOURCE_GRAPH, "search") => "Searching for jobs",
        (SOURCE_GRAPH, "challenge" | "challenge_page") => "Checking the page",
        (SOURCE_GRAPH, "dedup") => "Dropping jobs already known",
        (SOURCE_GRAPH, "next_candidate" | "fetch") => "Reading a posting",
        (SOURCE_GRAPH, "filter") => "Checking a job against your preferences",
        (SOURCE_GRAPH, "score") => "Scoring a job",
        (SOURCE_GRAPH, "insight") => "Reading about the company",
        (SOURCE_GRAPH, "save_lead" | "dismiss") => "Saving a job",
        (SOURCE_GRAPH, "give_up" | "finish") => "Finishing",

        (PREPARE_GRAPH, "promote") => "Creating the application",
        (PREPARE_GRAPH, "generate") => "Writing the CV",

        (SUGGEST_GRAPH, "plan") => "Planning the searches",
        (SUGGEST_GRAPH, "search") => "Searching the web",
        (SUGGEST_GRAPH, "verify") => "Checking the pages found",
        (SUGGEST_GRAPH, "save") => "Saving the suggestions",

        (INSIGHT_GRAPH, "company_urls") => "Choosing the company's pages",
        (INSIGHT_GRAPH, "fetch_pages") => "Reading the company's pages",
        (INSIGHT_GRAPH, "extract") => "Writing the brief",

        (TRENDS_GRAPH, "plan") => "Planning the sources",
        (TRENDS_GRAPH, "hackernews" | "yc" | "feeds") => "Reading the sources",
        (TRENDS_GRAPH, "store_items") => "Storing the items",
        (TRENDS_GRAPH, "wait_human") => "Checking for sources that need you",
        (TRENDS_GRAPH | TRENDS_DIGEST_GRAPH, "extract") => "Reading the items with your AI provider",
        (TRENDS_GRAPH | TRENDS_DIGEST_GRAPH, "count") => "Counting skills",
        (TRENDS_GRAPH | TRENDS_DIGEST_GRAPH, "synthesize") => "Writing the digest",
        (TRENDS_GRAPH | TRENDS_DIGEST_GRAPH, "enforce_citations") => "Checking the citations",
        (TRENDS_GRAPH | TRENDS_DIGEST_GRAPH, "save_report") => "Saving the digest",

        (_, "wait_human") => "Checking for pages that need you",
        _ => return None,
    };
    Some(label)
}

/// The status line of a running thread of `graph` at `nodes`: the first of them that has a label.
pub fn step_label<N: AsRef<str>>(graph: &str, nodes: &[N]) -> &'static str {
    match nodes {
        [] => STARTING,
        nodes => nodes.iter().find_map(|n| node_label(graph, n.as_ref())).unwrap_or(WORKING),
    }
}

/// What a paused thread needs from the user.
pub fn waiting_status(reason: &WaitReason) -> &'static str {
    match reason {
        WaitReason::Login { .. } => "Waiting for you: sign in",
        WaitReason::Captcha { .. } => "Waiting for you: captcha",
        WaitReason::Blocked { .. } => "Waiting for you: the site blocked JobPilot",
        WaitReason::UserMessage | WaitReason::ConfirmPreferences | WaitReason::Unknown => "Waiting for you",
    }
}

/// The period of a `trends:<period>` or `trends-digest:<period>` thread, as its label.
fn trends_period(job: &Job) -> Option<&str> {
    match job.graph.as_str() {
        TRENDS_GRAPH => job.thread.strip_prefix("trends:"),
        TRENDS_DIGEST_GRAPH => job.thread.strip_prefix("trends-digest:"),
        _ => None,
    }
}

fn title(job: &Job) -> String {
    match job.graph.as_str() {
        DISCOVERY_GRAPH => "Job search".to_string(),
        SOURCE_GRAPH => match job.source.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) => format!("Job search: {name}"),
            None => "Job search: a source".to_string(),
        },
        PREPARE_GRAPH => "Preparing a CV".to_string(),
        SUGGEST_GRAPH => "Finding job sources".to_string(),
        INSIGHT_GRAPH => "Reading about the company".to_string(),
        TRENDS_GRAPH | TRENDS_DIGEST_GRAPH => match trends_period(job) {
            Some(label) => {
                let period = Period::parse(label).map_or_else(|| label.to_string(), period_title);
                format!("Market research: {period}")
            }
            None => "Market research".to_string(),
        },
        _ => "Background job".to_string(),
    }
}

fn screen(graph: &str) -> &'static str {
    match graph {
        TRENDS_GRAPH | TRENDS_DIGEST_GRAPH => "trends",
        SUGGEST_GRAPH => "settings",
        _ => "jobs",
    }
}

/// The list the sidebar shows, from every running and waiting thread: running jobs first, then
/// those waiting for the user, each group by thread id. The order does not depend on the one given
/// nor on how far a job is, so a line keeps its place while the jobs run.
///
/// - The preference chat is left out: it waits whenever a chat is open.
/// - A running source search is part of its "Job search" line while that whole run is listed;
///   it gets its own line when it runs alone (a source being continued). A waiting one always
///   has its own line: it needs the user.
/// - A company brief is left out while a job search runs: it is one of its steps.
/// - A period's collection and its digest are one "Market research" line (the running one wins).
pub fn activity(jobs: &[Job]) -> Vec<ActivityDto> {
    let runs = |graph: &str| jobs.iter().any(|j| j.waiting.is_none() && j.graph == graph);
    let searching = runs(DISCOVERY_GRAPH) || runs(SOURCE_GRAPH);
    let parent_listed = |job: &Job| {
        parse_source_thread(&job.thread).is_some_and(|(run, _)| {
            let parent = format!("discover:{run}");
            jobs.iter().any(|j| j.waiting.is_none() && j.graph == DISCOVERY_GRAPH && j.thread == parent)
        })
    };
    let shown = |job: &Job| match job.graph.as_str() {
        PREFS_CHAT_GRAPH => false,
        SOURCE_GRAPH => job.waiting.is_some() || !parent_listed(job),
        INSIGHT_GRAPH => job.waiting.is_some() || !searching,
        _ => true,
    };

    let mut seen_periods: Vec<&str> = Vec::new();
    let mut listed = Vec::new();
    let mut ordered: Vec<&Job> = jobs.iter().filter(|j| shown(j)).collect();
    ordered.sort_by(|a, b| (a.waiting.is_some(), &a.thread).cmp(&(b.waiting.is_some(), &b.thread)));
    for job in ordered {
        if let Some(period) = trends_period(job) {
            if seen_periods.contains(&period) {
                continue;
            }
            seen_periods.push(period);
        }
        let (status, state) = match &job.waiting {
            Some(reason) => (waiting_status(reason), "waiting"),
            None => (step_label(&job.graph, &job.nodes), "running"),
        };
        listed.push(ActivityDto {
            id: job.thread.clone(),
            title: title(job),
            status: status.to_string(),
            state,
            screen: screen(&job.graph),
        });
    }
    listed
}

#[cfg(test)]
mod tests {
    use jobpilot_engine::graph::WaitReason;
    use jobpilot_engine::trends::Period;

    use super::{activity, period_title, step_label, waiting_status, ActivityDto, Job};

    fn running(graph: &str, thread: &str, nodes: &[&str]) -> Job {
        Job {
            thread: thread.into(),
            graph: graph.into(),
            nodes: nodes.iter().map(|n| (*n).to_string()).collect(),
            waiting: None,
            source: None,
        }
    }

    fn waiting(graph: &str, thread: &str, reason: WaitReason) -> Job {
        Job {
            waiting: Some(reason),
            ..running(graph, thread, &["wait_human"])
        }
    }

    fn login() -> WaitReason {
        WaitReason::Login {
            source_id: 7,
            url: "https://jobs.test/login".into(),
        }
    }

    fn lines(list: &[ActivityDto]) -> Vec<(&str, &str, &str)> {
        list.iter()
            .map(|a| (a.id.as_str(), a.title.as_str(), a.status.as_str()))
            .collect()
    }

    #[test]
    fn every_graph_has_a_title_a_status_and_a_screen() {
        let list = activity(&[
            running("discovery", "discover:r1", &["sources"]),
            running("prepare_cv", "lead:4", &["generate"]),
            running("source_suggest", "source_suggest", &["verify"]),
            running("trends", "trends:2026-W46", &["hackernews", "yc", "feeds"]),
            running("something_new", "x", &["step"]),
        ]);
        assert_eq!(
            lines(&list),
            [
                ("discover:r1", "Job search", "Searching the sources"),
                ("lead:4", "Preparing a CV", "Writing the CV"),
                (
                    "source_suggest",
                    "Finding job sources",
                    "Checking the pages found"
                ),
                (
                    "trends:2026-W46",
                    "Market research: Week of 9 November",
                    "Reading the sources"
                ),
                ("x", "Background job", "Working"),
            ]
        );
        assert_eq!(
            list.iter().map(|a| a.screen).collect::<Vec<_>>(),
            ["jobs", "jobs", "settings", "trends", "jobs"]
        );
        assert!(list.iter().all(|a| a.state == "running"));
    }

    #[test]
    fn a_job_says_starting_before_its_first_node_and_working_at_an_unknown_one() {
        let list = activity(&[
            running("prepare_cv", "lead:4", &[]),
            running("insight", "insight-1-0", &["router"]),
        ]);
        assert_eq!(list[0].title, "Reading about the company");
        assert_eq!(list[0].status, "Working");
        assert_eq!(list[1].status, "Starting");
        assert_eq!(step_label("prepare_cv", &["promote"]), "Creating the application");
        assert_eq!(step_label("prepare_cv", &["extract"]), "Working", "labels are per graph");
        // A fan-out: the first node with a label speaks.
        let fan = activity(&[running("source_run", "discover:r1:3", &["fan", "score", "insight"])]);
        assert_eq!(fan[0].status, "Scoring a job");
    }

    #[test]
    fn a_running_source_is_part_of_its_search_but_a_waiting_one_has_its_own_line() {
        let named = Job {
            source: Some("Acme careers".into()),
            ..waiting("source_run", "discover:r1:7", login())
        };
        let list = activity(&[
            running("discovery", "discover:r1", &["sources"]),
            running("source_run", "discover:r1:3", &["fetch"]),
            named,
            running("insight", "insight-1-0", &["extract"]),
        ]);
        assert_eq!(
            lines(&list),
            [
                ("discover:r1", "Job search", "Searching the sources"),
                (
                    "discover:r1:7",
                    "Job search: Acme careers",
                    "Waiting for you: sign in"
                ),
            ]
        );
        assert_eq!((list[1].state, list[1].screen), ("waiting", "jobs"));

        // A source continued on its own (its run is over) is listed, by name when it has one.
        let alone = activity(&[
            running("source_run", "discover:r1:3", &["fetch"]),
            running("discovery", "discover:r2", &["plan"]),
        ]);
        assert_eq!(
            lines(&alone)[0],
            ("discover:r1:3", "Job search: a source", "Reading a posting")
        );
        assert_eq!(alone.len(), 2);

        // A brief read outside a search (a pasted link) is listed.
        let brief = activity(&[running("insight", "insight-1-0", &["fetch_pages"])]);
        assert_eq!(
            lines(&brief),
            [(
                "insight-1-0",
                "Reading about the company",
                "Reading the company's pages"
            )]
        );
    }

    #[test]
    fn the_preference_chat_is_never_listed() {
        let list = activity(&[
            waiting("prefs_chat", "prefs", WaitReason::UserMessage),
            running("prefs_chat", "prefs", &["ask"]),
        ]);
        assert!(list.is_empty());
    }

    #[test]
    fn a_period_is_one_line_and_the_running_thread_wins() {
        let list = activity(&[
            waiting("trends", "trends:2026-Q4", login()),
            running("trends_digest", "trends-digest:2026-Q4", &["synthesize"]),
            running("trends", "trends:2026-M11", &["extract"]),
        ]);
        assert_eq!(
            lines(&list),
            [
                (
                    "trends-digest:2026-Q4",
                    "Market research: Q4 2026",
                    "Writing the digest"
                ),
                (
                    "trends:2026-M11",
                    "Market research: November 2026",
                    "Reading the items with your AI provider"
                ),
            ]
        );
        // Alone, the paused collection says what it needs; an unreadable period keeps its label.
        let paused = activity(&[
            waiting("trends", "trends:2026-Q4", WaitReason::Captcha { source_id: 1, url: String::new() }),
            running("trends", "trends:someday", &["plan"]),
        ]);
        assert_eq!(
            lines(&paused),
            [
                ("trends:someday", "Market research: someday", "Planning the sources"),
                ("trends:2026-Q4", "Market research: Q4 2026", "Waiting for you: captcha"),
            ]
        );
        assert_eq!(paused[1].screen, "trends");
    }

    #[test]
    fn the_order_is_running_then_waiting_each_by_thread_whatever_order_is_given() {
        let jobs = [
            waiting("trends", "trends:2026-Q4", login()),
            running("prepare_cv", "lead:9", &["generate"]),
            waiting("source_run", "discover:r1:7", login()),
            running("discovery", "discover:r2", &["plan"]),
            running("prepare_cv", "lead:4", &["promote"]),
        ];
        let ids = |list: &[ActivityDto]| list.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        let expected = ["discover:r2", "lead:4", "lead:9", "discover:r1:7", "trends:2026-Q4"];
        assert_eq!(ids(&activity(&jobs)), expected);
        let mut reversed = jobs.to_vec();
        reversed.reverse();
        assert_eq!(ids(&activity(&reversed)), expected);
    }

    #[test]
    fn the_trends_steps_read_as_on_the_trends_screen() {
        // The labels of `src/lib/trends/jobs.ts` (`stepLabel`), word for word.
        for (node, label) in [
            ("plan", "Planning the sources"),
            ("hackernews", "Reading the sources"),
            ("yc", "Reading the sources"),
            ("feeds", "Reading the sources"),
            ("store_items", "Storing the items"),
            ("wait_human", "Checking for sources that need you"),
            ("extract", "Reading the items with your AI provider"),
            ("count", "Counting skills"),
            ("synthesize", "Writing the digest"),
            ("enforce_citations", "Checking the citations"),
            ("save_report", "Saving the digest"),
        ] {
            assert_eq!(step_label("trends", &[node]), label, "{node}");
        }
        assert_eq!(step_label("trends_digest", &["synthesize"]), "Writing the digest");
        assert_eq!(step_label("source_run", &["wait_human"]), "Checking for pages that need you");
    }

    #[test]
    fn waiting_says_what_is_needed() {
        let blocked = WaitReason::Blocked { source_id: 1, url: String::new() };
        assert_eq!(waiting_status(&login()), "Waiting for you: sign in");
        assert_eq!(waiting_status(&blocked), "Waiting for you: the site blocked JobPilot");
        assert_eq!(waiting_status(&WaitReason::Unknown), "Waiting for you");
    }

    #[test]
    fn periods_are_named() {
        let title = |label: &str| Period::parse(label).map(period_title);
        assert_eq!(title("2026-W46").as_deref(), Some("Week of 9 November"));
        assert_eq!(title("2026-M11").as_deref(), Some("November 2026"));
        assert_eq!(title("2026").as_deref(), Some("2026"));
    }

    #[test]
    fn the_dto_is_camel_case_json() {
        let list = activity(&[running("prepare_cv", "lead:4", &["promote"])]);
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            serde_json::json!([{
                "id": "lead:4",
                "title": "Preparing a CV",
                "status": "Creating the application",
                "state": "running",
                "screen": "jobs",
            }])
        );
    }
}
