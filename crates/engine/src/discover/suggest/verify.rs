//! The verify step of "Suggest sources": a candidate the search found becomes a suggestion only
//! after the same check a source the user adds goes through has read it and found jobs.
//!
//! **The seam.** The graph only knows [`VerifySource`]. The running app's implementation is
//! [`live_verifier`], which calls [`Core::check_job_source`] (the check of "Add source": a board
//! must answer, a job page must list postings) and counts the postings whose title fits the wanted
//! roles ([`matching_titles`]). Tests pass a fake.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::discover::source::title_fits_roles;
use crate::Core;

/// A candidate the check accepted: the source as the check resolved it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Verified {
    /// The kind to add (a job page that turned out to be a board's page has the board's kind).
    pub kind: String,
    /// The address to add.
    pub url: String,
    /// The name the check read (the board's company, the page's host).
    pub name: String,
    /// Postings found.
    pub jobs: u32,
    /// Of those, postings whose title fits the wanted roles (all of them when no role is set).
    pub matching: u32,
}

/// Checks one candidate (`kind`, `input` as found on a results page). `Err` when it is not a job
/// source (no such board, no postings on the page, a login or captcha in the way, the browser
/// cannot be used): the graph drops it and logs a count, never the address.
#[async_trait]
pub trait VerifySource: Send + Sync {
    async fn verify(&self, kind: &str, input: &str) -> Result<Verified>;
}

/// How many of `titles` fit `roles` (the rule discovery uses: a meaningful word of a role in the
/// title). With no roles every title fits.
pub fn matching_titles<'a>(titles: impl IntoIterator<Item = &'a str>, roles: &[String]) -> u32 {
    let count = titles.into_iter().filter(|t| title_fits_roles(t, roles)).count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The verifier of the running app: [`Core::check_job_source`] with no session (a job page is read
/// in a Private tab, a board through its API), with `matching` from [`matching_titles`] over the
/// postings' titles and the saved `Preferences::roles`.
pub(crate) fn live_verifier(core: &Core) -> Arc<dyn VerifySource> {
    Arc::new(CheckJobSource { core: Arc::new(core.handles()) })
}

struct CheckJobSource {
    core: Arc<Core>,
}

#[async_trait]
impl VerifySource for CheckJobSource {
    async fn verify(&self, kind: &str, input: &str) -> Result<Verified> {
        let checked = self.core.check_job_source(kind, input, None).await?;
        let roles = self.core.preferences()?.roles;
        Ok(Verified {
            matching: matching_titles(checked.titles.iter().map(String::as_str), &roles),
            jobs: u32::try_from(checked.jobs).unwrap_or(u32::MAX),
            kind: checked.kind,
            url: checked.url,
            name: checked.name,
        })
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use anyhow::bail;

    use super::*;

    /// Answers by input: a [`Verified`] or an error text; anything else is "no postings found".
    #[derive(Default)]
    pub(crate) struct FakeVerify {
        answers: HashMap<String, Result<Verified, String>>,
        asked: Mutex<Vec<String>>,
    }

    impl FakeVerify {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// `input` checks out as a source of `kind` named `name` with `jobs` postings, `matching` of
        /// them fitting.
        pub(crate) fn ok(mut self, kind: &str, input: &str, name: &str, jobs: u32, matching: u32) -> Self {
            let v = Verified { kind: kind.into(), url: input.into(), name: name.into(), jobs, matching };
            self.answers.insert(input.into(), Ok(v));
            self
        }

        /// The check of `input` resolves to another source (a job page that is a board's).
        pub(crate) fn resolved(mut self, input: &str, to: Verified) -> Self {
            self.answers.insert(input.into(), Ok(to));
            self
        }

        pub(crate) fn fails(mut self, input: &str, error: &str) -> Self {
            self.answers.insert(input.into(), Err(error.into()));
            self
        }

        /// The inputs checked so far, in order.
        pub(crate) fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    #[async_trait]
    impl VerifySource for FakeVerify {
        async fn verify(&self, _kind: &str, input: &str) -> Result<Verified> {
            self.asked.lock().unwrap_or_else(|e| e.into_inner()).push(input.to_string());
            match self.answers.get(input) {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => bail!("{e}"),
                None => bail!("no job postings found on this page"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_titles_follow_the_role_rule() {
        let roles = vec!["Backend Engineer".to_string()];
        let titles = ["Senior Backend Engineer", "Marketing Manager", "Engineering Lead", "Office assistant"];
        assert_eq!(matching_titles(titles, &roles), 2);
        assert_eq!(matching_titles(titles, &[]), 4, "no roles: everything fits");
    }
}
