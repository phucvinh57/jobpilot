//! Sample data for UI work (`JOBPILOT_DEMO=1`): a profile read from one note, applications, job
//! sources, scored leads, a company brief, trends and the sites the user is signed in to (Settings >
//! Browser; never checked, so no browser is needed). Every address is on `example.com`; nothing
//! is fetched.

use anyhow::Result;
use chrono::{Duration, Utc};
use jobpilot_engine::discover::dedup::dedup_key;
use jobpilot_engine::discover::lead_dir;
use jobpilot_engine::discover::score::{Criteria, Criterion, Eligible, Scored};
use jobpilot_engine::discover::{RunSummary, LAST_RUN_KEY};
use jobpilot_engine::pipeline::cv::Cv;
use jobpilot_engine::pipeline::insight::{CompanyFacts, Interview, Skills};
use jobpilot_engine::pipeline::Insight;
use jobpilot_engine::prefs::Weights;
use jobpilot_engine::store::{
    LeadState, NewApplication, NewLead, NewSession, NewSource, SourcePurpose, Status,
};
use jobpilot_engine::Core;

/// Insert a few sample applications when the store is empty (`JOBPILOT_DEMO=1`).
pub fn seed(core: &Core) -> Result<()> {
    seed_demo_profile(core)?;
    jobpilot_engine::prefs::seed_demo(core)?;
    crate::trends::demo::seed_demo(core)?;
    seed_demo_applications(core)?;
    seed_demo_sessions(core)?;
    seed_demo_queue(core)
}

/// Two sites the user is signed in to (Settings > Browser): one checked, one not yet.
fn seed_demo_sessions(core: &Core) -> Result<()> {
    if !core.store.list_sessions()?.is_empty() {
        return Ok(());
    }
    let jobs = core.store.add_session(&NewSession {
        name: "Example Jobs".into(),
        start_url: "https://jobs.example.com/feed".into(),
    })?;
    core.store.update_session_domains(jobs.id, r#"["example.com","jobs.example.com"]"#)?;
    core.store.update_session_status(jobs.id, "ok", Some(Utc::now() - Duration::hours(3)))?;
    core.store.add_session(&NewSession {
        name: "Example Careers".into(),
        start_url: "https://careers.example.com/account".into(),
    })?;
    Ok(())
}

/// The note the demo profile is "read" from (Profile > About you).
const DEMO_NOTE: &str = "Minh Tran, senior backend engineer. Ho Chi Minh City, Vietnam. minh.tran@example.com\n\
Lotus Pay (2021 to present): senior backend engineer, payment services in Rust and PostgreSQL, Docker and Nginx on Linux, CI/CD on AWS.\n\
Saigon Market (2018 to 2021): backend engineer, PHP and Python APIs, Go workers.\n\
BSc Computer Science, Ho Chi Minh City University of Technology, 2014 to 2018.\n\
Open source: tinyqueue, a Redis-backed job queue in Go.\n\
English (professional), Vietnamese (native).";

/// The profile the demo shows, as if read from [`DEMO_NOTE`]. Its skills match what the demo
/// leads and trends assume (`trends::demo::PROFILE`: Rust, Python, PostgreSQL, Go, PHP).
const DEMO_PROFILE_YAML: &str = r#"
basics:
  name: Minh Tran
  headline: Senior backend engineer
  location: Ho Chi Minh City, Vietnam
  email: minh.tran@example.com
summary:
  - Backend engineer building payment services in Rust and PostgreSQL.
skills:
  - category: Languages
    items: [Rust, Python, Go, PHP]
  - category: Data
    items: [PostgreSQL, Redis]
  - category: Infrastructure
    items: [Linux, Docker, Nginx, AWS, CI/CD]
experience:
  - company: Lotus Pay
    position: Senior Backend Engineer
    location: Ho Chi Minh City
    start: "2021"
    end: Present
    bullets:
      - Payment services in Rust and PostgreSQL, run with Docker and Nginx on Linux.
  - company: Saigon Market
    position: Backend Engineer
    location: Ho Chi Minh City
    start: "2018"
    end: "2021"
    bullets:
      - APIs in PHP and Python, background workers in Go.
projects:
  - name: tinyqueue
    intro: A Redis-backed job queue in Go.
education:
  - institution: Ho Chi Minh City University of Technology
    degree: BSc Computer Science
    start: "2014"
    end: "2018"
languages:
  - language: English
    fluency: Professional
  - language: Vietnamese
    fluency: Native
"#;

/// A note and the profile read from it, when the user has neither yet. The note comes first:
/// the profile is stored as read from the documents that exist at that moment.
fn seed_demo_profile(core: &Core) -> Result<()> {
    if core.profile()?.is_some() || !core.profile_sources()?.is_empty() {
        return Ok(());
    }
    core.add_profile_text("Demo CV", DEMO_NOTE)?;
    let left_out = ["Certification “CKA”", "Role “Tech Lead” at “Lotus Pay”"].map(String::from);
    core.save_profile_cv(&Cv::parse(DEMO_PROFILE_YAML)?, &left_out)
}

fn seed_demo_applications(core: &Core) -> Result<()> {
    if !core.store.list_applications()?.is_empty() {
        return Ok(());
    }
    let samples = [
        (
            "acme-backend-engineer",
            "Acme",
            "Backend Engineer",
            Status::CvReady,
        ),
        (
            "foo-platform-engineer",
            "Foo",
            "Platform Engineer",
            Status::Applied,
        ),
        (
            "bar-devops-engineer",
            "Bar",
            "DevOps Engineer",
            Status::Interview,
        ),
        ("baz-sre", "Baz", "Site Reliability Engineer", Status::Draft),
        (
            "qux-backend-developer",
            "Qux",
            "Backend Developer",
            Status::Offer,
        ),
        (
            "zed-platform-engineer",
            "Zed",
            "Platform Engineer",
            Status::Rejected,
        ),
    ];
    for (slug, company, role, status) in samples {
        let app = core.store.create_application(&NewApplication {
            slug: slug.into(),
            company: company.into(),
            role: role.into(),
            source: Some(format!("https://{slug}.example.com/jobs/1")),
            dir: core.paths.application_dir(slug),
            ..Default::default()
        })?;
        core.store.force_status(app.id, status, "demo data")?;
        if slug == "bar-devops-engineer" {
            core.store
                .set_interview_round(app.id, Some("Technical interview"))?;
        }
        if slug == "acme-backend-engineer" {
            let json = serde_json::to_string(&demo_insight(slug))?;
            core.store.set_insight_json(app.id, Some(&json))?;
        }
    }
    Ok(())
}

/// Job sources and scored leads for the queue (skipped when the queue has something already).
fn seed_demo_queue(core: &Core) -> Result<()> {
    if !core.store.list_sources(SourcePurpose::Jobs)?.is_empty()
        || !core.store.list_leads(LeadState::New)?.is_empty()
    {
        return Ok(());
    }
    let source = |kind: &str, name: &str, url: &str| {
        core.store.add_source(&NewSource {
            kind: kind.into(),
            name: name.into(),
            url: url.into(),
            purpose: SourcePurpose::Jobs,
            session_id: None,
        })
    };
    let greenhouse = source(
        "greenhouse",
        "Acme careers",
        "https://boards.greenhouse.io/acme",
    )?;
    let lever = source("lever", "Foo jobs", "https://jobs.lever.co/foo")?;
    let hn = source(
        "hn_hiring",
        "HN: Who is hiring",
        "https://news.ycombinator.com/item?id=1",
    )?;
    core.store.set_source_run(greenhouse.id, "ok")?;
    core.store.set_source_run(lever.id, "ok")?;
    core.store.set_source_status(hn.id, "needs_human")?;

    // (source, company, role, location, [skills, domain, location, salary, seniority])
    const US_ONLY: &str = "Remote - US";
    type Row = (u32, bool, &'static str);
    let jobs: [(i64, &str, &str, &str, [Row; 5]); 7] = [
        (
            greenhouse.id,
            "Acme",
            "Senior Backend Engineer",
            "Singapore",
            [
                (
                    92,
                    true,
                    "Rust, PostgreSQL and distributed systems are all in your profile.",
                ),
                (
                    80,
                    true,
                    "Payments infrastructure is close to your fintech interest.",
                ),
                (70, true, "Hybrid in Singapore, one of your cities."),
                (0, false, "No salary stated."),
                (90, true, "Senior, as you look for."),
            ],
        ),
        (
            lever.id,
            "Foo",
            "Platform Engineer",
            "Remote (APAC)",
            [
                (
                    78,
                    true,
                    "Kubernetes and Linux match; Terraform is not in your profile.",
                ),
                (55, true, "Developer tools, not a listed domain."),
                (100, true, "Fully remote."),
                (60, true, "Pay range is near your minimum."),
                (80, true, "Mid to senior."),
            ],
        ),
        (
            greenhouse.id,
            "Acme",
            "Site Reliability Engineer",
            "Ho Chi Minh City, Vietnam",
            [
                (70, true, "Linux, Nginx and observability match."),
                (80, true, "Same company as the payments role."),
                (100, true, "On-site in Ho Chi Minh City, where you live."),
                (0, false, "No salary stated."),
                (60, true, "Mid-level title."),
            ],
        ),
        (
            hn.id,
            "Quark Labs",
            "Staff Systems Engineer",
            "Berlin, Germany",
            [
                (
                    60,
                    true,
                    "C++ and Rust asked; embedded work is not in your profile.",
                ),
                (30, true, "Robotics is an avoided domain."),
                (20, true, "On-site in Berlin."),
                (0, false, "No salary stated."),
                (40, true, "Staff level is above your experience."),
            ],
        ),
        (
            lever.id,
            "Bar",
            "DevOps Engineer",
            "Remote",
            [
                (66, true, "Docker, CI/CD and AWS match."),
                (50, true, "E-commerce is neutral."),
                (100, true, "Fully remote."),
                (0, false, "No salary stated."),
                (65, true, "Mid-level."),
            ],
        ),
        (
            hn.id,
            "Nimbus",
            "Backend Developer",
            "",
            [
                (0, false, "The posting is too short to judge."),
                (0, false, "No domain stated."),
                (0, false, "No location stated."),
                (0, false, "No salary stated."),
                (0, false, "No level stated."),
            ],
        ),
        // Hires in the US only: with "Can work from" set to Vietnam, it is hidden from the queue.
        (
            lever.id,
            "Initech",
            "Senior Rust Engineer",
            US_ONLY,
            [
                (85, true, "Rust, PostgreSQL and Kubernetes match."),
                (50, true, "Enterprise software is neutral."),
                (90, true, "Fully remote."),
                (0, false, "No salary stated."),
                (85, true, "Senior, as you look for."),
            ],
        ),
    ];
    let weights = Weights::default();
    for (i, (source_id, company, role, location, rows)) in jobs.into_iter().enumerate() {
        let criterion = |(score, known, reason): Row| Criterion {
            score,
            known,
            reason: reason.into(),
        };
        let [skills, domain, loc, salary, seniority] = rows.map(criterion);
        let criteria = Criteria {
            skills,
            domain,
            location: loc,
            salary,
            seniority,
        };
        // Only the US-only posting says where it hires; the others leave it open.
        let (eligible, eligible_reason) = if location == US_ONLY {
            (Eligible::No, "hires in the United States only".to_string())
        } else {
            (Eligible::Unclear, String::new())
        };
        let scored = Scored {
            total: criteria.weighted(&weights),
            criteria,
            weights,
            eligible,
            eligible_reason,
            // The demo preferences name no work_from: this "no" counts only after a new score.
            judged_work_from: Vec::new(),
        };
        let slug = format!("{}-{i}", company.to_lowercase().replace(' ', "-"));
        let url = format!("https://jobs.example.com/{slug}");
        let lead = core.store.add_lead(&NewLead {
            source_id: Some(source_id),
            dedup_key: dedup_key(company, role),
            url: url.clone(),
            company: company.into(),
            role: role.into(),
            location: location.into(),
            score: scored.total,
            score_json: scored.total.map(|_| scored.to_json()).transpose()?,
            insight_json: (i % 3 == 0)
                .then(|| serde_json::to_string(&demo_insight(&slug)))
                .transpose()?,
            ..NewLead::default()
        })?;
        let dir = lead_dir(&core.paths, lead.id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("jd.md");
        let text = format!(
            "# {company} \u{2014} {role}\nSource: {url}\n\nWe are hiring a {role} at {company}. You will design, build and run backend services, \
             own their reliability, and work with a small team. Requirements: Linux, networking, a systems language and clear English.\n"
        );
        std::fs::write(&path, text)?;
        core.store.set_lead_jd_path(lead.id, &path)?;
    }
    let summary = RunSummary {
        run: "demo".into(),
        started_at: (Utc::now() - Duration::minutes(12)).to_rfc3339(),
        finished_at: (Utc::now() - Duration::minutes(10)).to_rfc3339(),
        saved: 6,
        waiting: 1,
        ..RunSummary::default()
    };
    core.store
        .kv_set(LAST_RUN_KEY, &serde_json::to_string(&summary)?)?;
    Ok(())
}

/// A company brief for the demo data: some facts found, some not.
fn demo_insight(slug: &str) -> Insight {
    let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Insight {
        domain: "B2B payments infrastructure".into(),
        business_summary: "Sells a payments API to online marketplaces, which use it to pay out sellers in 40 countries.".into(),
        company: CompanyFacts {
            size: "200-500 employees".into(),
            stage: "Series C".into(),
            hq: "Singapore".into(),
            founded: "2016".into(),
            ..CompanyFacts::default()
        },
        skills: Skills {
            required: strings(&["Rust", "PostgreSQL", "Distributed systems", "Linux"]),
            preferred: strings(&["Kafka", "Kubernetes"]),
        },
        interview: Interview {
            stages: strings(&["Recruiter call", "Technical screen", "System design", "Team interview", "Offer"]),
            notes: "The whole process takes about three weeks.".into(),
            found: true,
        },
        sources: vec![
            format!("https://{slug}.example.com/jobs/1"),
            format!("https://{slug}.example.com/about"),
            format!("https://{slug}.example.com/careers"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_engine::store::Status;
    use jobpilot_engine::{Core, Paths, Secrets};

    #[test]
    fn the_sample_applications_cover_every_stage_once() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).expect("a core");
        super::seed_demo_applications(&core).expect("the samples");
        super::seed_demo_applications(&core).expect("seeding again is a no-op");
        let apps = core.store.list_applications().expect("the applications");
        assert_eq!(apps.len(), 6);
        let status = |slug: &str| apps.iter().find(|a| a.slug == slug).map(|a| (a.status, a.interview_round.clone()));
        assert_eq!(status("bar-devops-engineer"), Some((Status::Interview, Some("Technical interview".into()))));
        assert_eq!(status("qux-backend-developer"), Some((Status::Offer, None)));
        assert_eq!(status("zed-platform-engineer"), Some((Status::Rejected, None)));
    }

    #[test]
    fn the_demo_profile_is_read_from_its_note_once_and_every_fact_traces_to_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).expect("a core");
        super::seed_demo_profile(&core).expect("the profile");
        super::seed_demo_profile(&core).expect("seeding again is a no-op");
        assert_eq!(core.profile_sources().expect("the documents").len(), 1);
        let profile = core.require_profile().expect("a profile");
        assert!(!profile.stale, "read from the note that is there");
        assert_eq!(profile.left_out.len(), 2);

        let cv = jobpilot_engine::pipeline::cv::Cv::parse(super::DEMO_PROFILE_YAML).expect("valid");
        let (traced, left_out) = jobpilot_engine::profile::trace::trace(cv.clone(), super::DEMO_NOTE);
        assert!(left_out.is_empty(), "{left_out:#?}");
        assert_eq!(traced, cv);
    }
}
