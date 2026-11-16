//! Choosing which memories to show.
//!
//! Four stages, and none of them call a model:
//!
//! 1. **Recall** runs three cheap channels and keeps a wide pool. Precision here
//!    is not the goal; keeping the answer in the pool is.
//! 2. **Fusion** combines the channels by reciprocal rank. The channels disagree
//!    about scale, so only their order is trusted.
//! 3. **Ranking** scores each candidate on evidence a personal message actually
//!    provides: what the channels said, whether the message named its subject,
//!    what kind of statement it is, whether it is still true, and how old it is.
//! 4. **Fill** keeps the best-scored candidates up to the item cap, so a long
//!    decisive fact is not displaced by a short one.

use std::collections::{HashMap, HashSet};

use crate::model::{Kind, Memory};

/// The fusion constant. 60 is the value from the original reciprocal-rank work
/// and is what the benchmark was tuned against.
pub const RRF_K: f32 = 60.0;
/// The widest pool the channels may contribute.
pub const POOL: usize = 128;
/// At most this many rows share a kind unless the message named their subject.
const KIND_DIVERSITY: usize = 2;
/// Two rows this similar say the same thing.
const DUPLICATE: f32 = 0.9;

/// Weights for the ranking features. They are hand-set from the benchmark, not
/// learned: a personal store has no relevance labels to train on.
mod weight {
    pub const RELEVANCE: f32 = 1.0;
    pub const ENTITY: f32 = 0.70;
    pub const COVERAGE: f32 = 0.80;
    pub const KIND: f32 = 0.25;
    pub const CURRENT: f32 = 0.35;
    pub const RECENCY: f32 = 0.10;
}

/// One scored candidate.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub memory: Memory,
    pub score: f32,
}

/// Reciprocal-rank fusion over several ranked channels.
///
/// A channel that did not return an id contributes nothing, which is what makes
/// fusion robust to a channel being unavailable: the vector channel simply
/// disappears from the sum when no model is installed.
pub fn fuse(channels: &[Vec<String>]) -> Vec<(String, f32)> {
    let mut scores: HashMap<String, f32> = HashMap::new();
    for channel in channels {
        for (rank, id) in channel.iter().enumerate() {
            let contribution = 1.0 / (RRF_K + rank as f32 + 1.0);
            *scores.entry(id.clone()).or_insert(0.0) += contribution;
        }
    }
    let mut fused: Vec<(String, f32)> = scores.into_iter().collect();
    fused.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    fused
}

/// How much a statement is still the current answer.
fn recency(updated_at: i64, now: i64) -> f32 {
    const YEAR_MICROS: f32 = 365.0 * 86_400.0 * 1_000_000.0;
    let age = (now - updated_at).max(0) as f32;
    (-age / (2.0 * YEAR_MICROS)).exp().clamp(0.0, 1.0)
}

/// Whether a memory's subject is one the message named.
fn entity_match(memory: &Memory, entities: &HashSet<String>) -> f32 {
    if memory.subject.is_empty() {
        return 0.0;
    }
    if entities.contains(&memory.subject) {
        return 1.0;
    }
    entities
        .iter()
        .any(|entity| {
            entity.starts_with(&format!("{} ", memory.subject))
                || memory.subject.starts_with(&format!("{entity} "))
        })
        .into()
}

/// The words of a statement, folded the way the index folds them.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether one query term is present in a statement.
///
/// The comparison is prefix in either direction, which is the cheap stand-in for
/// a stemmer: it matches `queued` against `queue` and `deploy` against
/// `deployment` without a dictionary. Terms shorter than three characters are
/// exact, because a prefix that short matches almost anything.
fn contains(term: &str, words: &[String]) -> bool {
    words.iter().any(|word| {
        word == term
            || (term.chars().count() >= 3
                && word.chars().count() >= 3
                && (word.starts_with(term) || term.starts_with(word.as_str())))
    })
}

/// The share of the message's own words a statement answers.
///
/// This is the strongest cheap relevance signal there is. A statement that
/// carries four of the user's five content words is almost always the answer,
/// whatever the fusion score says.
fn coverage(memory: &Memory, terms: &[String]) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let words = words(&format!("{} {}", memory.subject, memory.content));
    let matched = terms.iter().filter(|term| contains(term, &words)).count();
    matched as f32 / terms.len() as f32
}

/// Score the fused pool.
pub fn rank(
    fused: &[(String, f32)],
    memories: &HashMap<String, Memory>,
    entities: &HashSet<String>,
    terms: &[String],
    now: i64,
) -> Vec<Candidate> {
    let best = fused.first().map(|(_, score)| *score).unwrap_or(0.0);
    let best = if best > 0.0 { best } else { 1.0 };
    let mut candidates = Vec::with_capacity(fused.len());
    for (id, fused_score) in fused {
        let Some(memory) = memories.get(id) else {
            continue;
        };
        let relevance = fused_score / best;
        let score = weight::RELEVANCE * relevance
            + weight::ENTITY * entity_match(memory, entities)
            + weight::COVERAGE * coverage(memory, terms)
            + weight::KIND * memory.kind.prior()
            + weight::CURRENT * f32::from(memory.is_current())
            + weight::RECENCY * recency(memory.updated_at, now);
        candidates.push(Candidate {
            memory: memory.clone(),
            score,
        });
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.memory.id.cmp(&right.memory.id))
    });
    candidates
}

/// Drop rows that repeat each other or crowd one kind.
pub fn diversify(candidates: Vec<Candidate>, entities: &HashSet<String>) -> Vec<Candidate> {
    let mut kept: Vec<Candidate> = Vec::with_capacity(candidates.len());
    let mut per_kind: HashMap<Kind, usize> = HashMap::new();
    let mut kept_ids: Vec<String> = Vec::new();

    for candidate in candidates {
        let memory = &candidate.memory;
        // A superseded row repeats what its replacement says.
        if kept_ids
            .iter()
            .any(|id| memory.superseded_by.as_deref() == Some(id))
        {
            continue;
        }
        if kept.iter().any(|other| {
            crate::store::similarity(&other.memory.content, &memory.content) >= DUPLICATE
        }) {
            continue;
        }
        let anchored = entity_match(memory, entities) > 0.0;
        let count = per_kind.entry(memory.kind).or_insert(0);
        if !anchored && *count >= KIND_DIVERSITY {
            continue;
        }
        *count += 1;
        kept_ids.push(memory.id.clone());
        kept.push(candidate);
        if kept.len() >= POOL {
            break;
        }
    }
    kept
}

/// Keep the best `max_items` candidates.
///
/// The item cap is the only bound. Ordering by score keeps a long, decisive
/// statement from being displaced by a shorter one at a lower score.
pub fn fill(candidates: Vec<Candidate>, max_items: usize) -> Vec<Memory> {
    let mut scored = candidates;
    scored.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.memory.id.cmp(&right.memory.id))
    });
    scored
        .into_iter()
        .take(max_items)
        .map(|candidate| candidate.memory)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn memory(id: &str, kind: Kind, subject: &str, content: &str) -> Memory {
        Memory {
            id: id.to_owned(),
            kind,
            content: content.to_owned(),
            subject: subject.to_owned(),
            pinned: false,
            source: Source::Agent,
            source_entry: None,
            valid_from: 0,
            valid_to: None,
            superseded_by: None,
            created_at: 0,
            updated_at: 0,
            revision: 1,
        }
    }

    #[test]
    fn fusion_rewards_a_row_two_channels_agree_on() {
        let lexical = vec!["a".to_owned(), "b".to_owned()];
        let vector = vec!["b".to_owned(), "c".to_owned()];
        let fused = fuse(&[lexical, vector]);
        assert_eq!(fused[0].0, "b", "a row both channels found must lead");
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn a_named_subject_outranks_a_bare_keyword_hit() {
        let fused = vec![("kw".to_owned(), 1.0), ("named".to_owned(), 0.9)];
        let mut memories = HashMap::new();
        memories.insert("kw".to_owned(), memory("kw", Kind::Fact, "", "alice"));
        memories.insert(
            "named".to_owned(),
            memory("named", Kind::Person, "alice", "a fact"),
        );
        let entities: HashSet<String> = ["alice".to_owned()].into_iter().collect();
        let ranked = rank(&fused, &memories, &entities, &["alice".to_owned()], 0);
        assert_eq!(ranked[0].memory.id, "named");
    }

    #[test]
    fn a_kind_crowd_is_trimmed_unless_the_subject_is_named() {
        let mut memories = HashMap::new();
        let fused: Vec<(String, f32)> = (0..5)
            .map(|index| {
                let id = format!("f{index}");
                memories.insert(
                    id.clone(),
                    memory(&id, Kind::Fact, "other", &format!("fact number {index}")),
                );
                (id, 1.0 - index as f32 / 10.0)
            })
            .collect();
        let ranked = rank(&fused, &memories, &HashSet::new(), &["fact".to_owned()], 0);
        assert_eq!(diversify(ranked, &HashSet::new()).len(), KIND_DIVERSITY);
    }

    #[test]
    fn a_long_high_scoring_statement_is_not_displaced_by_a_shorter_one() {
        let mut candidates = vec![Candidate {
            memory: memory("long", Kind::Fact, "", &"x".repeat(400)),
            score: 1.0,
        }];
        for index in 0..10 {
            candidates.push(Candidate {
                memory: memory(&format!("short{index}"), Kind::Fact, "", "a short fact"),
                score: 0.95,
            });
        }
        let chosen = fill(candidates, 10);
        assert_eq!(chosen.len(), 10);
        assert!(
            chosen.iter().any(|memory| memory.id == "long"),
            "a long statement at the highest score must keep its slot"
        );
    }
}
