//! Nodes shared by several graphs.

use serde::{Deserialize, Serialize};

use super::WaitReason;

/// What stands between a source and its pages (found by `browser/challenge.rs`, later).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeKind {
    /// The site wants the user to log in.
    Login,
    /// A captcha is shown.
    Captcha,
    /// The site blocks the app (403, 429, block page).
    Blocked,
}

/// A detected login, captcha or block page, kept in the graph's state until the user has dealt
/// with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    pub kind: ChallengeKind,
    /// The `sources` row the page belongs to.
    pub source_id: i64,
    /// The page the user has to open.
    pub url: String,
}

impl Challenge {
    /// What the paused thread waits for.
    pub fn wait_reason(&self) -> WaitReason {
        let (source_id, url) = (self.source_id, self.url.clone());
        match self.kind {
            ChallengeKind::Login => WaitReason::Login { source_id, url },
            ChallengeKind::Captcha => WaitReason::Captcha { source_id, url },
            ChallengeKind::Blocked => WaitReason::Blocked { source_id, url },
        }
    }
}

/// A graph state that can hold a [`Challenge`].
pub trait Challenged {
    /// The challenge detected by an earlier node, if any.
    fn challenge(&self) -> Option<&Challenge>;
}

/// The shared interrupt of discovery and trends: pause while the state holds a [`Challenge`].
///
/// ```ignore
/// graph.wait("wait_human", nodes::challenge)
/// // the UI's "Continue" clears it:
/// graphs.resume(&graph, thread, |s| s.challenge = None).await
/// ```
pub fn challenge<S: Challenged>(state: &S) -> Option<WaitReason> {
    state.challenge().map(Challenge::wait_reason)
}
