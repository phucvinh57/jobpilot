//! Sample data for UI work (`JOBPILOT_DEMO=1`): applications, job sources, scored leads, a
//! company brief and trends. Every address is on `example.com`; nothing is fetched.

use anyhow::Result;
use jobpilot_core::chrono::{Duration, Utc};
use jobpilot_core::discover::dedup::dedup_key;
use jobpilot_core::discover::lead_dir;
use jobpilot_core::discover::score::{Criteria, Criterion, Scored};
use jobpilot_core::discover::{RunSummary, LAST_RUN_KEY};
use jobpilot_core::pipeline::insight::{CompanyFacts, Interview, Skills};
use jobpilot_core::pipeline::Insight;
use jobpilot_core::prefs::Weights;
use jobpilot_core::store::{LeadState, NewApplication, NewLead, NewSource, SourcePurpose, Status};
use jobpilot_core::Core;

/// Insert a few sample applications when the store is empty (`JOBPILOT_DEMO=1`).
pub fn seed(core: &Core) -> Result<()> {
    jobpilot_core::prefs::seed_demo(core)?;
    crate::trends::demo::seed_demo(core)?;
    seed_demo_applications(core)?;
    seed_demo_queue(core)
}

fn seed_demo_applications(core: &Core) -> Result<()> {
    if !core.store.list_applications()?.is_empty() {
        return Ok(());
    }
    let samples = [
        ("acme-backend-engineer", "Acme", "Backend Engineer", Status::CvReady),
        ("foo-platform-engineer", "Foo", "Platform Engineer", Status::Applied),
        ("bar-devops-engineer", "Bar", "DevOps Engineer", Status::Interview),
        ("baz-sre", "Baz", "Site Reliability Engineer", Status::Draft),
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
        if slug == "acme-backend-engineer" {
            let json = serde_json::to_string(&demo_insight(slug))?;
            core.store.set_insight_json(app.id, Some(&json))?;
        }
    }
    Ok(())
}

/// Job sources and scored leads for the queue (skipped when the queue has something already).
fn seed_demo_queue(core: &Core) -> Result<()> {
    if !core.store.list_sources(SourcePurpose::Jobs)?.is_empty() || !core.store.list_leads(LeadState::New)?.is_empty() {
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
    let greenhouse = source("greenhouse", "Acme careers", "https://boards.greenhouse.io/acme")?;
    let lever = source("lever", "Foo jobs", "https://jobs.lever.co/foo")?;
    let hn = source("hn_hiring", "HN: Who is hiring", "https://news.ycombinator.com/item?id=1")?;
    core.store.set_source_run(greenhouse.id, "ok")?;
    core.store.set_source_run(lever.id, "ok")?;
    core.store.set_source_status(hn.id, "needs_human")?;

    // (source, company, role, location, [skills, domain, location, salary, seniority])
    type Row = (u32, bool, &'static str);
    let jobs: [(i64, &str, &str, &str, [Row; 5]); 6] = [
        (greenhouse.id, "Acme", "Senior Backend Engineer", "Singapore", [
            (92, true, "Rust, PostgreSQL and distributed systems are all in your profile."),
            (80, true, "Payments infrastructure is close to your fintech interest."),
            (70, true, "Hybrid in Singapore, one of your cities."),
            (0, false, "No salary stated."),
            (90, true, "Senior, as you look for."),
        ]),
        (lever.id, "Foo", "Platform Engineer", "Remote (APAC)", [
            (78, true, "Kubernetes and Linux match; Terraform is not in your profile."),
            (55, true, "Developer tools, not a listed domain."),
            (100, true, "Fully remote."),
            (60, true, "Pay range is near your minimum."),
            (80, true, "Mid to senior."),
        ]),
        (greenhouse.id, "Acme", "Site Reliability Engineer", "Ho Chi Minh City, Vietnam", [
            (70, true, "Linux, Nginx and observability match."),
            (80, true, "Same company as the payments role."),
            (100, true, "On-site in Ho Chi Minh City, where you live."),
            (0, false, "No salary stated."),
            (60, true, "Mid-level title."),
        ]),
        (hn.id, "Quark Labs", "Staff Systems Engineer", "Berlin, Germany", [
            (60, true, "C++ and Rust asked; embedded work is not in your profile."),
            (30, true, "Robotics is an avoided domain."),
            (20, true, "On-site in Berlin."),
            (0, false, "No salary stated."),
            (40, true, "Staff level is above your experience."),
        ]),
        (lever.id, "Bar", "DevOps Engineer", "Remote", [
            (66, true, "Docker, CI/CD and AWS match."),
            (50, true, "E-commerce is neutral."),
            (100, true, "Fully remote."),
            (0, false, "No salary stated."),
            (65, true, "Mid-level."),
        ]),
        (hn.id, "Nimbus", "Backend Developer", "", [
            (0, false, "The posting is too short to judge."),
            (0, false, "No domain stated."),
            (0, false, "No location stated."),
            (0, false, "No salary stated."),
            (0, false, "No level stated."),
        ]),
    ];
    let weights = Weights::default();
    for (i, (source_id, company, role, location, rows)) in jobs.into_iter().enumerate() {
        let criterion = |(score, known, reason): Row| Criterion { score, known, reason: reason.into() };
        let [skills, domain, loc, salary, seniority] = rows.map(criterion);
        let criteria = Criteria { skills, domain, location: loc, salary, seniority };
        let scored = Scored { total: criteria.weighted(&weights), criteria, weights };
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
            insight_json: (i % 3 == 0).then(|| serde_json::to_string(&demo_insight(&slug))).transpose()?,
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
    core.store.kv_set(LAST_RUN_KEY, &serde_json::to_string(&summary)?)?;
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
