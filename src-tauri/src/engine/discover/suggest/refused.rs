//! Candidates the check refused lately (no such board, no postings, a wall), kept for
//! [`REFUSED_DAYS`] so the next searches spend their check budget on other candidates. Stored as
//! one kv row (`discover.suggest.refused`): `{ kind, input, at }` entries, newest last, at most
//! [`MAX_REFUSED`]. A failure the user can fix (browser, network) is never recorded.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::store::Store;

const KEY: &str = "discover.suggest.refused";
/// How long a refused candidate is skipped.
pub const REFUSED_DAYS: i64 = 30;
/// Most entries kept (the oldest go first).
const MAX_REFUSED: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Refused {
    pub kind: String,
    pub input: String,
    pub at: DateTime<Utc>,
}

/// The entries younger than [`REFUSED_DAYS`] at `now`. An unreadable row reads as empty.
pub(crate) fn recent(store: &Store, now: DateTime<Utc>) -> Result<Vec<Refused>> {
    let Some(raw) = store.kv_get(KEY)? else { return Ok(Vec::new()) };
    let all: Vec<Refused> = serde_json::from_str(&raw).unwrap_or_else(|_| {
        tracing::warn!("the refused suggestions list could not be read, starting over");
        Vec::new()
    });
    let since = now - Duration::days(REFUSED_DAYS);
    Ok(all.into_iter().filter(|r| r.at > since).collect())
}

/// Add `refused` (kind, input) pairs at `now`, keeping the recent ones and the cap.
pub(crate) fn record(store: &Store, refused: &[(String, String)], now: DateTime<Utc>) -> Result<()> {
    if refused.is_empty() {
        return Ok(());
    }
    let mut all = recent(store, now)?;
    all.retain(|r| !refused.iter().any(|(kind, input)| r.kind == *kind && r.input == *input));
    all.extend(refused.iter().map(|(kind, input)| Refused { kind: kind.clone(), input: input.clone(), at: now }));
    let skip = all.len().saturating_sub(MAX_REFUSED);
    store.kv_set(KEY, &serde_json::to_string(&all[skip..])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Core, Paths, Secrets};

    #[test]
    fn refused_candidates_are_kept_for_a_while_then_forgotten_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        let now = Utc::now();
        assert!(recent(&core.store, now).unwrap().is_empty());
        record(&core.store, &[("lever".into(), "jobs.lever.co/a".into())], now - Duration::days(REFUSED_DAYS + 1)).unwrap();
        record(&core.store, &[("greenhouse".into(), "boards.greenhouse.io/b".into())], now).unwrap();
        let kept: Vec<_> = recent(&core.store, now).unwrap().into_iter().map(|r| r.input).collect();
        assert_eq!(kept, ["boards.greenhouse.io/b"], "the old one is forgotten");

        let many: Vec<(String, String)> = (0..MAX_REFUSED + 5).map(|i| ("career_page".to_string(), format!("https://c{i}.test/jobs"))).collect();
        record(&core.store, &many, now).unwrap();
        let kept = recent(&core.store, now).unwrap();
        assert_eq!(kept.len(), MAX_REFUSED);
        assert_eq!(kept.last().unwrap().input, format!("https://c{}.test/jobs", MAX_REFUSED + 4), "the newest stay");

        core.store.kv_set(KEY, "not json").unwrap();
        assert!(recent(&core.store, now).unwrap().is_empty());
    }
}
