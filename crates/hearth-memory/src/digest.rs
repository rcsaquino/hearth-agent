//! Rendering memory into the prompt.
//!
//! Two blocks, both small on purpose.
//!
//! **Rules** sit in the cached prefix. They are few, they change rarely, and they
//! are the reason a "never do X" instruction survives a long conversation without
//! being retrieved.
//!
//! **Recall** sits immediately before the newest user message, the one place in
//! the prompt that is never cached. Every token there is paid in full on every
//! turn, so a recall line is terse: identity, subject, age, statement. Coverage
//! is stated explicitly, because a digest that silently shows six of fourteen
//! facts teaches the model that six is all there is.

use std::collections::BTreeMap;

use crate::model::Memory;

/// Longest statement shown in one recall line.
const LINE_CHARS: usize = 140;
/// Longest statement shown in one rule line.
///
/// A rule is capped at this length when it is written, so the render and the
/// budget check measure the same text. The clip only protects a store that was
/// edited outside the tool path.
const RULE_CHARS: usize = 400;

/// How many subjects the coverage line names before it says "and more".
const COVERAGE_SUBJECTS: usize = 3;

/// What the message named, and how much more the store holds about it.
#[derive(Debug, Clone, Default)]
pub struct Coverage {
    /// Current memories per named subject, including the ones shown.
    pub current: BTreeMap<String, usize>,
    /// Earlier versions per named subject.
    pub earlier: BTreeMap<String, usize>,
}

impl Coverage {
    pub fn is_empty(&self) -> bool {
        self.current.is_empty() && self.earlier.is_empty()
    }
}

fn clip(text: &str, limit: usize) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut clipped: String = flattened.chars().take(limit).collect();
    if flattened.chars().count() > limit {
        clipped.push('…');
    }
    clipped
}

fn age(updated_at: i64, now: i64) -> String {
    let days = ((now - updated_at).max(0) / 86_400_000_000).max(0);
    match days {
        0 => "now".to_owned(),
        days if days < 30 => format!("{days}d"),
        days if days < 365 => format!("{}mo", days / 30),
        days => format!("{}y", days / 365),
    }
}

/// The shortest unambiguous name for a kind.
fn kind_code(memory: &Memory) -> &'static str {
    use crate::model::Kind;
    match memory.kind {
        Kind::Rule => "rule",
        Kind::Preference => "pref",
        Kind::Person => "person",
        Kind::Decision => "decision",
        Kind::Commitment => "commit",
        Kind::Episode => "episode",
        Kind::Fact => "fact",
    }
}

/// One line of the recall digest.
pub fn line(memory: &Memory, now: i64) -> String {
    let mut line = String::with_capacity(LINE_CHARS + 32);
    line.push_str(&memory.id);
    line.push(' ');
    if memory.kind != crate::model::Kind::Fact {
        line.push_str(kind_code(memory));
        line.push(' ');
    }
    if !memory.subject.is_empty() {
        line.push_str(&memory.subject);
        line.push(' ');
    }
    line.push_str(&age(memory.updated_at, now));
    line.push_str(": ");
    line.push_str(&clip(&memory.content, LINE_CHARS));
    line
}

/// What one recall line costs, in estimated tokens.
///
/// The per-line overhead is the id, kind, subject, age, and separator. The fill
/// spends the statement budget on this cost, and [`coverage_cost`] reserves what
/// the coverage lines need, so the render cannot overshoot.
pub fn line_cost(memory: &Memory) -> u64 {
    let content = clip(&memory.content, LINE_CHARS);
    let metadata = 14
        + memory.id.len()
        + if memory.subject.is_empty() {
            0
        } else {
            memory.subject.len()
        };
    hearth_model::estimate_tokens(&content).max(1) as u64 + (metadata as u64) / 4
}

/// Render the recall digest.
///
/// The tags belong to the prompt layer, which emits one segment per layer. This
/// returns the body only, and collapses to an empty string when nothing was
/// recalled so a prompt never announces an empty section.
pub fn recall_body(items: &[Memory], coverage: &Coverage, now: i64) -> String {
    if items.is_empty() && coverage.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = items.iter().map(|memory| line(memory, now)).collect();
    lines.extend(coverage_lines(items, coverage));
    lines.join("\n")
}

/// The coverage line, when the digest shows a subset.
fn coverage_lines(items: &[Memory], coverage: &Coverage) -> Vec<String> {
    let shown: BTreeMap<&str, usize> = items.iter().fold(BTreeMap::new(), |mut counts, memory| {
        if !memory.subject.is_empty() {
            *counts.entry(memory.subject.as_str()).or_insert(0) += 1;
        }
        counts
    });
    let mut remaining: Vec<String> = coverage
        .current
        .iter()
        .filter_map(|(subject, total)| {
            let hidden = total.saturating_sub(shown.get(subject.as_str()).copied().unwrap_or(0));
            (hidden > 0).then(|| format!("{subject} {hidden}"))
        })
        .take(COVERAGE_SUBJECTS)
        .collect();
    let earlier: Vec<String> = coverage
        .earlier
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(subject, count)| format!("{subject} {count}"))
        .take(COVERAGE_SUBJECTS)
        .collect();

    let mut lines = Vec::new();
    if !remaining.is_empty() {
        remaining.sort();
        lines.push(format!(
            "more: {} | memory_recall(subject) for the rest",
            remaining.join(", ")
        ));
    }
    if !earlier.is_empty() {
        lines.push(format!(
            "earlier: {} | memory_recall(as_of) for what was true then",
            earlier.join(", ")
        ));
    }
    lines
}

/// Render the rules block body.
///
/// Ordering is by creation, not by update, so a rule that is edited in place does
/// not reorder the rest and the prefix hash only moves when the text moves. The
/// tags belong to the prompt layer.
pub fn rules_body(rules: &[Memory], max_tokens: u64) -> String {
    if rules.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::with_capacity(rules.len());
    let mut used = 0u64;
    for rule in rules {
        let line = rule_line(&rule.id, &rule.content);
        let cost = hearth_model::estimate_tokens(&line).max(1) as u64;
        if used + cost > max_tokens {
            break;
        }
        used += cost;
        lines.push(line);
    }
    if lines.len() < rules.len() {
        lines.push(format!(
            "{} more rules were not shown.",
            rules.len() - lines.len()
        ));
    }
    lines.join("\n")
}

/// What the coverage lines cost when no statement is shown.
///
/// The fill spends only what is left, so a long subject cannot push the
/// rendered digest past its budget.
pub fn coverage_cost(coverage: &Coverage) -> u64 {
    let text = coverage_lines(&[], coverage).join("\n");
    if text.is_empty() {
        0
    } else {
        hearth_model::estimate_tokens(&text).max(1) as u64
    }
}

/// Drop one coverage subject, longest name first. Returns false when empty.
pub fn trim_coverage(coverage: &mut Coverage) -> bool {
    let current = coverage.current.keys().max_by_key(|key| key.len()).cloned();
    let earlier = coverage.earlier.keys().max_by_key(|key| key.len()).cloned();
    let drop_current = match (&current, &earlier) {
        (Some(current), Some(earlier)) => current.len() >= earlier.len(),
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => return false,
    };
    if drop_current {
        current.is_some_and(|key| coverage.current.remove(&key).is_some())
    } else {
        earlier.is_some_and(|key| coverage.earlier.remove(&key).is_some())
    }
}

/// The token cost of rendering a rule line.
pub fn rule_cost(memory: &Memory) -> u64 {
    hearth_model::estimate_tokens(&rule_line(&memory.id, &memory.content)).max(1) as u64
}

/// The token cost of a rule line that does not exist yet.
///
/// The write-time check and the render must measure the same line, so the
/// placeholder id has the shape of a real one.
pub fn rule_cost_text(content: &str) -> u64 {
    hearth_model::estimate_tokens(&rule_line(crate::store::ID_SAMPLE, content)).max(1) as u64
}

fn rule_line(id: &str, content: &str) -> String {
    format!("{}: {}", id, clip(content, RULE_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Source};

    fn memory(id: &str, subject: &str, content: &str) -> Memory {
        Memory {
            id: id.to_owned(),
            kind: Kind::Fact,
            content: content.to_owned(),
            subject: subject.to_owned(),
            pinned: false,
            source: Source::User,
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
    fn a_line_is_identity_subject_age_and_statement() {
        let line = line(&memory("m_1", "alice", "Alice manages the OR board."), 0);
        assert_eq!(line, "m_1 alice now: Alice manages the OR board.");
    }

    #[test]
    fn an_empty_recall_renders_nothing() {
        assert!(recall_body(&[], &Coverage::default(), 0).is_empty());
    }

    #[test]
    fn coverage_names_only_the_subjects_with_rows_left_out() {
        let items = vec![memory("m_1", "alice", "one")];
        let coverage = Coverage {
            current: [("alice".to_owned(), 7), ("bob".to_owned(), 1)].into(),
            earlier: BTreeMap::new(),
        };
        let block = recall_body(&items, &coverage, 0);
        assert!(block.contains("alice 6"), "{block}");
        assert!(
            block.contains("bob 1"),
            "a subject with nothing shown is still worth reporting: {block}"
        );
    }

    #[test]
    fn an_earlier_version_is_reported_separately_from_a_live_one() {
        let coverage = Coverage {
            current: [("lab".to_owned(), 1)].into(),
            earlier: [("lab".to_owned(), 1)].into(),
        };
        let block = recall_body(
            &[memory("m_1", "lab", "the lab is in building 9")],
            &coverage,
            0,
        );
        assert!(block.contains("earlier: lab 1"), "{block}");
        assert!(!block.contains("more:"), "nothing is hidden: {block}");
    }

    #[test]
    fn the_rules_block_stops_at_its_budget() {
        let rules: Vec<Memory> = (0..20)
            .map(|index| memory(&format!("m_{index}"), "", &"x".repeat(200)))
            .collect();
        let block = rules_body(&rules, 200);
        assert!(hearth_model::estimate_tokens(&block) <= 240, "{block}");
        assert!(block.contains("more rules were not shown"), "{block}");
    }
}
