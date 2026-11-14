//! Sample chat and preferences for `JOBPILOT_DEMO=1` (UI work and screenshots).

use anyhow::Result;

use super::chat::Question;
use super::chat_graph::{QUESTIONS_KEY, THREAD};
use super::{Preferences, Remote, Weights};
use crate::llm::Role;
use crate::Core;

/// Seed a short conversation, preferences and an undo point when nothing was saved or said yet.
pub fn seed_demo(core: &Core) -> Result<()> {
    if core.store.has_preferences()? || !core.store.chat_messages(THREAD)?.is_empty() {
        return Ok(());
    }
    let chat = [
        (Role::Assistant, "Hi! I'll ask a few questions to learn what job you are looking for.\n\n- Which roles are you aiming for?\n- Do you want to work remotely?"),
        (Role::User, "Backend or platform engineer, senior level. Remote preferred, hybrid in Ho Chi Minh City is fine."),
        (Role::Assistant, "Got it: backend and platform roles, senior, remote or hybrid in Ho Chi Minh City.\n\n- Which skills must a job ask for?\n- Any minimum salary?"),
        (Role::User, "Rust and Kubernetes are a must. I'd like at least 4000 USD a month, no crypto please."),
        (Role::Assistant, "Noted: Rust and Kubernetes required, crypto excluded.\n\n- Which company stage do you prefer?"),
    ];
    for (role, text) in chat {
        core.store.add_chat_message(THREAD, role, text)?;
    }
    let questions = vec![Question {
        text: "Which company stage do you prefer?".into(),
        options: ["Seed", "Series A-B", "Growth", "Public"].map(String::from).to_vec(),
    }];
    core.store.kv_set(QUESTIONS_KEY, &serde_json::to_string(&questions)?)?;

    let first = Preferences {
        roles: vec!["Backend Engineer".into(), "Platform Engineer".into()],
        seniority: vec!["Senior".into()],
        remote: Remote::Remote,
        ..Default::default()
    };
    let first = core.update_preferences(first)?;
    // A second change, so "Undo last change" has something to undo.
    core.update_preferences(Preferences {
        must_skills: vec!["Rust".into(), "Kubernetes".into()],
        nice_skills: vec!["PostgreSQL".into(), "Terraform".into()],
        avoid_domains: vec!["Crypto".into()],
        locations: vec!["Ho Chi Minh City".into()],
        remote: Remote::Hybrid,
        salary_min: Some(48_000),
        currency: Some("USD".into()),
        languages: vec!["English".into(), "Vietnamese".into()],
        weights: Weights { skills: 40, domain: 15, location: 15, salary: 15, seniority: 15 },
        ..first
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Paths, Secrets};

    #[test]
    fn seeds_once_and_leaves_something_to_undo() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        seed_demo(&core).unwrap();
        let view = core.prefs_chat_view().unwrap();
        assert_eq!(view.messages.len(), 5);
        assert_eq!(view.questions.len(), 1);
        assert!(view.saved && view.can_undo);
        assert_eq!(view.status, super::super::chat_graph::ChatStatus::Idle, "no live thread: a message starts one");
        assert_eq!(view.preferences.weights.sum(), 100);

        let before = view.preferences.clone();
        seed_demo(&core).unwrap();
        assert_eq!(core.prefs_chat_view().unwrap().preferences, before);
        assert_eq!(core.store.chat_messages(THREAD).unwrap().len(), 5, "a second call adds nothing");
    }
}
