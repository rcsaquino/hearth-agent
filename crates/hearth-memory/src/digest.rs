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
///
/// A new memory is capped at this length, so it is shown whole. The clip only
/// protects a row written before that limit.
const LINE_CHARS: usize = 150;
/// The read-side guard for a rule line.
///
/// A rule written before the 150-character limit used to be capped at this
/// length. The clip only protects an older store.
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
    let lines: Vec<String> = rules
        .iter()
        .map(|rule| rule_line(&rule.id, &rule.content))
        .collect();
    fill_rule_lines(&lines, max_tokens).1
}

/// Whether every rule, including one that is not stored yet, fits the render.
///
/// The write check and the prompt must measure the same block, or a write can
/// be admitted that the render then has to hide behind a notice.
pub fn rules_fit(existing: &[&Memory], content: &str, max_tokens: u64) -> bool {
    let mut lines: Vec<String> = existing
        .iter()
        .map(|rule| rule_line(&rule.id, &rule.content))
        .collect();
    lines.push(rule_line(crate::store::ID_SAMPLE, content));
    fill_rule_lines(&lines, max_tokens).0 == lines.len()
}

/// The token cost of rendering every rule, with no notice.
pub fn rules_cost(existing: &[&Memory]) -> u64 {
    let text = existing
        .iter()
        .map(|rule| rule_line(&rule.id, &rule.content))
        .collect::<Vec<_>>()
        .join("\n");
    cost_of_chars(text.chars().count() as u64)
}

/// Fill rule lines against a budget, reserving the notice when lines are left out.
///
/// Returns the number of lines shown and the rendered block. The block never
/// costs more than `max_tokens`; a budget too small for the notice renders nothing.
fn fill_rule_lines(lines: &[String], max_tokens: u64) -> (usize, String) {
    if lines.is_empty() {
        return (0, String::new());
    }
    let mut selected = 0usize;
    let mut chars = 0u64;
    for (index, line) in lines.iter().enumerate() {
        let line_chars = line.chars().count() as u64;
        let omitted = lines.len() - index - 1;
        // The newlines between lines, plus the newline and text of a notice
        // when this line would still leave rules out.
        let mut projected = chars + line_chars + selected as u64;
        if omitted > 0 {
            projected += 1 + omitted_notice(omitted).chars().count() as u64;
        }
        if cost_of_chars(projected) > max_tokens {
            break;
        }
        chars += line_chars;
        selected += 1;
    }
    if selected == lines.len() {
        return (selected, lines.join("\n"));
    }
    let notice = omitted_notice(lines.len() - selected);
    let mut rendered: Vec<&str> = lines[..selected].iter().map(String::as_str).collect();
    rendered.push(&notice);
    let block = rendered.join("\n");
    if cost_of_chars(block.chars().count() as u64) > max_tokens {
        // A budget smaller than the notice can name nothing without breaking
        // the bound it was given.
        return (0, String::new());
    }
    (selected, block)
}

/// The line that names the rules a budget left out.
fn omitted_notice(omitted: usize) -> String {
    format!("{omitted} more rules were not shown.")
}

/// The token cost of a rendered block, from its character count.
fn cost_of_chars(chars: u64) -> u64 {
    hearth_model::estimate_tokens_chars(chars).max(0) as u64
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
#[cfg(test)]
pub fn rule_cost(memory: &Memory) -> u64 {
    hearth_model::estimate_tokens(&rule_line(&memory.id, &memory.content)).max(1) as u64
}

/// The token cost of a rule line that does not exist yet.
///
/// The write-time check and the render must measure the same line, so the
/// placeholder id has the shape of a real one.
#[cfg(test)]
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
    fn a_memory_at_the_content_limit_is_shown_in_full() {
        let content = "a".repeat(crate::store::CONTENT_LIMIT);
        let line = line(&memory("m_1", "topic", &content), 0);
        assert!(line.ends_with(content.as_str()), "{line}");
        assert!(!line.contains('…'), "a new memory is not clipped: {line}");
    }

    #[test]
    fn an_older_row_above_the_content_limit_is_clipped() {
        let content = "b".repeat(crate::store::CONTENT_LIMIT + 10);
        let line = line(&memory("m_1", "topic", &content), 0);
        assert!(line.contains('…'), "an old row is clipped: {line}");
        assert!(!line.contains(content.as_str()), "{line}");
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
            .map(|index| memory(&format!("m_{index}"), "", &"x".repeat(228)))
            .collect();
        let block = rules_body(&rules, 200);
        assert!(hearth_model::estimate_tokens(&block) <= 200, "{block}");
        assert!(block.contains("more rules were not shown"), "{block}");
    }
}
