//! Which items the model reads. `count` has already measured every item of the period; the model
//! reads a capped pick (`extract` and `synthesize` cost a model call per few items, and the prompt
//! has a size), and it is the pick that its points can cite. So the pick is chosen to hold evidence
//! for what the counts found:
//!
//! 1. **indicators**, in order of importance ([`indicators`]): the skills whose direction is rising
//!    or falling, the skills of the gap, the terms not in the skill list, then the most mentioned
//!    skills. For each indicator in turn (round-robin, so no indicator takes the room of the
//!    others) the newest item of the period that is not yet picked and names it, until about half
//!    of the cap is used or no indicator has an item left. "Names it" is the skill vocabulary's
//!    match for a skill and a whole-word match for a term, on the item's own words
//!    ([`body`](super::terms::body)).
//! 2. **the rest** by [`spread`](super::spread): the other items round-robin over their sources,
//!    newest first, then the "Who is hiring" postings only in the room they leave. `count` counts
//!    every posting, and one backfilled Hacker News source holds thousands of them, which would
//!    crowd the stories, the directory and the feeds out of what `extract` reads.
//!
//! The indicator items come first in the result: when the evidence of the prompt is cut, the last
//! items go, and the ones that carry the numbers must stay. The pick is a pure function of the
//! store and the counts, so a node that runs again (a resume) picks the same items.

use std::collections::VecDeque;

use anyhow::Result;
use futures::StreamExt;

use super::count::{Vocabulary, TEXT_READS};
use super::terms::body;
use super::{period_items_all, spread, try_read_text, Direction, SkillCounts, Stratum};
use crate::discover::source::mentions_lowered;
use crate::store::{Store, TrendItem};
use crate::trends::period::Period;

/// The most mentioned skills that are indicators too.
const TOP_SKILLS: usize = 10;

/// Something the counts found that the model should be able to cite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Indicator {
    /// A skill of the vocabulary, by its index.
    Skill(usize),
    /// A term not in the skill list, in lower case.
    Term(String),
}

/// The indicators of `counts`, most important first, each once: the skills rising or falling (the
/// global scope, then the local), the skills of the gap, the listed terms, the most mentioned skills.
/// A skill the vocabulary cannot place is left out.
pub(super) fn indicators(counts: &SkillCounts, vocab: &Vocabulary) -> Vec<Indicator> {
    let mut found: Vec<Indicator> = Vec::new();
    let mut add = |indicator: Option<Indicator>| {
        if let Some(indicator) = indicator {
            if !found.contains(&indicator) {
                found.push(indicator);
            }
        }
    };
    let skill = |name: &str| vocab.find(name).first().copied().map(Indicator::Skill);
    for scope in std::iter::once(&counts.skills).chain(counts.local.as_ref().map(|local| &local.skills)) {
        for moving in scope.iter().filter(|s| matches!(s.direction, Direction::Rising | Direction::Falling)) {
            add(skill(&moving.skill));
        }
    }
    for gap in &counts.gap {
        add(skill(&gap.skill));
    }
    for term in &counts.uncatalogued {
        let term = term.term.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        add((!term.is_empty()).then_some(Indicator::Term(term)));
    }
    for top in counts.skills.iter().take(TOP_SKILLS) {
        add(skill(&top.skill));
    }
    found
}

/// The indicators (by their place in `indicators`) that an item's text names.
fn named(text: &str, stratum: Stratum, vocab: &Vocabulary, indicators: &[Indicator]) -> Vec<usize> {
    let own = body(text, stratum);
    let skills = vocab.skills_in(&own);
    let lower = own.to_lowercase();
    indicators
        .iter()
        .enumerate()
        .filter(|(_, indicator)| match indicator {
            Indicator::Skill(skill) => skills.contains(skill),
            Indicator::Term(term) => mentions_lowered(&lower, term),
        })
        .map(|(at, _)| at)
        .collect()
}

/// At most `max` items of `period` for the model to read (see the module docs). Without `counts`
/// (a digest that has not counted) there are no indicators and the pick is the plain spread. Reads
/// the text of every item of the period once, a few at a time, when there are indicators to look
/// for.
pub(super) async fn pick_items(store: &Store, vocab: &Vocabulary, period: &Period, max: usize, counts: Option<&SkillCounts>) -> Result<Vec<TrendItem>> {
    let items = period_items_all(store, period).await?;
    let indicators = counts.map(|counts| indicators(counts, vocab)).unwrap_or_default();
    let names: Vec<Vec<usize>> = if indicators.is_empty() {
        vec![Vec::new(); items.len()]
    } else {
        let owned: Vec<(TrendItem, Stratum)> = items.clone();
        futures::stream::iter(owned)
            .map(|(item, stratum)| {
                let indicators = &indicators;
                async move {
                    match try_read_text(&item).await {
                        Some(text) => named(&text, stratum, vocab, indicators),
                        None => Vec::new(),
                    }
                }
            })
            .buffered(TEXT_READS)
            .collect()
            .await
    };
    let total = items.len();
    let picked = choose(items, &names, indicators.len(), max);
    tracing::debug!(
        period_items = total,
        indicators = indicators.len(),
        indicator_items = names.iter().filter(|n| !n.is_empty()).count(),
        picked = picked.len(),
        "items for the model picked"
    );
    Ok(picked)
}

/// The pick, from the period's `items` (newest first, each with its stratum) and what each one
/// names (`names[i]` are the indicators, by place, that `items[i]` names; `indicators` of them).
/// The indicator part first, then the rest ([`spread`](super::spread)); at most `max` in all.
pub(super) fn choose(items: Vec<(TrendItem, Stratum)>, names: &[Vec<usize>], indicators: usize, max: usize) -> Vec<TrendItem> {
    // The items naming each indicator, newest first (the order of `items`).
    let mut queues: Vec<VecDeque<usize>> = vec![VecDeque::new(); indicators];
    for (at, named) in names.iter().enumerate().take(items.len()) {
        for &indicator in named {
            if let Some(queue) = queues.get_mut(indicator) {
                queue.push_back(at);
            }
        }
    }

    // Round-robin: one item per indicator in turn, until about half of the room is used.
    let budget = max.div_ceil(2);
    let mut taken = vec![false; items.len()];
    let mut first: Vec<usize> = Vec::new();
    'rounds: loop {
        let mut progress = false;
        for queue in &mut queues {
            if first.len() >= budget {
                break 'rounds;
            }
            while queue.front().is_some_and(|&at| taken.get(at).copied().unwrap_or(true)) {
                queue.pop_front();
            }
            if let Some(at) = queue.pop_front() {
                if let Some(slot) = taken.get_mut(at) {
                    *slot = true;
                }
                first.push(at);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }

    // The rest of the room: the other items, then postings in what they leave.
    let mut slots: Vec<Option<(TrendItem, Stratum)>> = items.into_iter().map(Some).collect();
    let mut picked: Vec<TrendItem> = first.iter().filter_map(|&at| slots.get_mut(at).and_then(Option::take)).map(|(item, _)| item).collect();
    let (postings, others): (Vec<_>, Vec<_>) = slots.into_iter().flatten().partition(|(_, stratum)| *stratum == Stratum::HiringPosting);
    let room = max.saturating_sub(picked.len());
    let others = spread(others.into_iter().map(|(item, _)| item).collect(), room);
    let room = room.saturating_sub(others.len());
    let postings = spread(postings.into_iter().map(|(item, _)| item).collect(), room);
    picked.extend(others);
    picked.extend(postings);
    picked
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::trends::digest::{GapBucket, GapSkill, ScopeCounts, SkillCount, TermCount};

    fn skill(name: &str, direction: Direction) -> SkillCount {
        SkillCount { skill: name.into(), by_month: vec![1], total: 1, direction, ..SkillCount::default() }
    }

    #[test]
    fn indicators_come_in_order_of_importance_each_once() {
        let vocab = Vocabulary::new(&[]);
        let id = |name: &str| Indicator::Skill(vocab.find(name)[0]);
        let counts = SkillCounts {
            skills: vec![
                skill("Kafka", Direction::Steady),
                skill("Rust", Direction::Rising),
                skill("Python", Direction::Falling),
                skill("Go", Direction::Unclear),
            ],
            local: Some(ScopeCounts { documents: vec![9], skills: vec![skill("Rust", Direction::Rising), skill("Elixir", Direction::Falling), skill("Docker", Direction::Steady)] }),
            gap: vec![
                GapSkill { skill: "Kafka".into(), bucket: GapBucket::LearnNext, direction: Direction::Steady, share_permille: 80, target_permille: 80, low_permille: 40, high_permille: 120 },
                GapSkill { skill: "Zig".into(), bucket: GapBucket::LearnNext, direction: Direction::Steady, share_permille: 60, target_permille: 60, low_permille: 30, high_permille: 100 },
                GapSkill { skill: "not a skill".into(), bucket: GapBucket::LearnNext, direction: Direction::Steady, share_permille: 60, target_permille: 60, low_permille: 30, high_permille: 100 },
            ],
            uncatalogued: vec![TermCount { term: "Mojo".into(), items: 9, ..TermCount::default() }, TermCount { term: " mojo ".into(), items: 8, ..TermCount::default() }],
            ..SkillCounts::default()
        };
        assert_eq!(
            indicators(&counts, &vocab),
            [
                id("Rust"),
                id("Python"),
                id("Elixir"),
                id("Kafka"),
                id("Zig"),
                Indicator::Term("mojo".into()),
                id("Go"),
            ],
            "rising and falling (global, then local), the gap, the terms, then the most mentioned"
        );
        assert!(indicators(&SkillCounts::default(), &vocab).is_empty());
    }

    #[test]
    fn what_an_item_names_is_read_on_its_own_words() {
        let vocab = Vocabulary::new(&[]);
        let rust = vocab.find("Rust")[0];
        let indicators = [Indicator::Skill(rust), Indicator::Term("event sourcing".into()), Indicator::Term("hacker".into())];
        let posting = "# Hacker News, Ask HN: Who is hiring? (November 2026)\n\nAcme | Rust | event-sourcing and Event Sourcing";
        assert_eq!(named(posting, Stratum::HiringPosting, &vocab, &indicators), [0, 1], "the thread's header is not the posting's words");
        assert_eq!(named("# T\n\nNothing here", Stratum::Feed, &vocab, &indicators), Vec::<usize>::new());
        assert_eq!(named("# Hacker news", Stratum::Feed, &vocab, &indicators), [2], "a feed item may say hacker");
    }

    fn item(id: i64, source: i64, age: i64) -> TrendItem {
        let at = Utc::now();
        TrendItem {
            id,
            source_id: Some(source),
            url: format!("https://x.test/{id}"),
            title: format!("t{id}"),
            published_at: Some(at - Duration::minutes(age)),
            text_path: None,
            fetched_at: at,
        }
    }

    /// `n` items, newest first: ids 1..=n, every third one a posting, the others from source 1 and 2.
    fn items(n: i64) -> Vec<(TrendItem, Stratum)> {
        (1..=n)
            .map(|id| {
                let stratum = if id % 3 == 0 { Stratum::HiringPosting } else { Stratum::Feed };
                (item(id, 1 + id % 2, id), stratum)
            })
            .collect()
    }

    fn ids(picked: &[TrendItem]) -> Vec<i64> {
        picked.iter().map(|i| i.id).collect()
    }

    #[test]
    fn indicators_are_served_round_robin_newest_first_up_to_half_of_the_room() {
        let items = items(12);
        // Item i is at index i - 1. Indicator 0 is named by items 9 (a posting), 6 and 2; indicator
        // 1 by 6 and 12; indicator 2 by nothing; indicator 3 by 11.
        let mut names = vec![Vec::new(); 12];
        for (indicator, named) in [(0, vec![9, 6, 2]), (1, vec![6, 12]), (3, vec![11])] {
            for id in named {
                names[id - 1].push(indicator);
            }
        }
        // Newest first within an indicator is the order of the items: 2 before 6 before 9.
        let picked = ids(&choose(items.clone(), &names, 4, 6));
        assert_eq!(picked.len(), 6);
        // Round 1: indicator 0 takes 2, indicator 1 takes 6, indicator 3 takes 11; round 2 would
        // go over the half (3 of 6), so the rest is the spread: the others (not postings) newest
        // first, then a posting only if there is room.
        assert_eq!(picked[..3], [2, 6, 11], "one item per indicator, the newest naming it");
        assert_eq!(picked[3..], [1, 4, 5], "the other items fill the room, newest first, the postings only after them");

        // With room for more, the second round goes on where the first stopped: indicator 0's next
        // is 9 (6 was taken for indicator 1), indicator 1's next is 12; no indicator has an item
        // left after that, although half of the room is not used yet.
        let picked = ids(&choose(items.clone(), &names, 4, 12));
        assert_eq!(picked, [2, 6, 11, 9, 12, 1, 4, 5, 7, 8, 10, 3], "the indicator items, then the others, then the one posting left");
    }

    #[test]
    fn an_old_posting_that_names_an_indicator_beats_newer_items_that_do_not() {
        // Plenty of newer non-postings, one old posting naming the indicator, room for 4.
        let mut items: Vec<(TrendItem, Stratum)> = (1..=8).map(|id| (item(id, 1, id), Stratum::Feed)).collect();
        items.push((item(9, 1, 500), Stratum::HiringPosting));
        let mut names = vec![Vec::new(); 9];
        names[8].push(0);
        let picked = ids(&choose(items.clone(), &names, 1, 4));
        assert_eq!(picked[0], 9, "the posting comes first");
        assert_eq!(picked.len(), 4);
        assert_eq!(picked[1..], [1, 2, 3], "then the newest of the others");
        // The old rule (no indicator) would have left every posting out.
        assert_eq!(ids(&choose(items, &vec![Vec::new(); 9], 0, 4)), [1, 2, 3, 4]);
    }

    #[test]
    fn without_indicators_the_rest_is_the_spread_with_postings_in_the_leftover_room() {
        let items = items(9);
        // Six others and three postings: room for 4 is four others, spread over the two sources.
        let picked = ids(&choose(items.clone(), &vec![Vec::new(); 9], 0, 4));
        assert_eq!(picked.len(), 4);
        assert!(items.iter().filter(|(i, s)| picked.contains(&i.id) && *s == Stratum::HiringPosting).count() == 0);
        // Room for 8: the six others and two postings, the newest of those.
        let picked = ids(&choose(items.clone(), &vec![Vec::new(); 9], 0, 8));
        assert_eq!(picked.len(), 8);
        assert_eq!(picked[6..], [3, 6], "postings last, newest first");
        // Room for all, or none.
        assert_eq!(choose(items.clone(), &vec![Vec::new(); 9], 0, 100).len(), 9);
        assert!(choose(items, &vec![Vec::new(); 9], 0, 0).is_empty());
    }

    #[test]
    fn the_pick_is_deterministic() {
        let items = items(30);
        let names: Vec<Vec<usize>> = (0..30).map(|i| (0..4).filter(|k| (i + k) % 3 == 0).collect()).collect();
        let a = ids(&choose(items.clone(), &names, 4, 10));
        let b = ids(&choose(items, &names, 4, 10));
        assert_eq!(a, b);
        let mut unique = a.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), a.len(), "an item is picked once");
        assert!(a.len() <= 10);
    }
}
