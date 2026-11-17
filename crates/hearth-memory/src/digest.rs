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
//! turn, so a recall line is terse: kind, subject, age, statement. Coverage is
//! stated explicitly, because a digest that silently shows six of fourteen facts
//! teaches the model that six is all there is.

use std::collections::{BTreeMap, HashSet};

use crate::model::Memory;

/// Longest statement shown in one recall line.
///
/// A new memory is capped at this length, so it is shown whole. The clip only
/// protects a row written before that limit.
const LINE_CHARS: usize = 200;
/// The read-side guard for a rule line.
///
/// A rule written before the 200-character limit used to be capped at this
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

/// One statement cut to a display limit, and whether anything was cut.
struct Clipped {
    text: String,
    truncated: bool,
}

fn clip(text: &str, limit: usize) -> Clipped {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated = flattened.chars().count() > limit;
    let mut text: String = flattened.chars().take(limit).collect();
    if truncated {
        text.push('…');
    }
    Clipped { text, truncated }
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

/// Where a row needs its id to be addressable.
#[derive(Clone, Copy)]
enum Addressing {
    /// Only a clipped statement needs the id, because the tool is the only way
    /// to read the rest.
    OnClip,
    /// The recall tool is where addressing belongs, so every row carries its id.
    Always,
}

/// One line of the recall digest.
pub fn line(memory: &Memory, now: i64) -> String {
    render(memory, now, Addressing::OnClip)
}

/// One line of the recall tool output.
pub fn tool_line(memory: &Memory, now: i64) -> String {
    render(memory, now, Addressing::Always)
}

/// Whether a line is a memory row rather than a coverage or rules notice.
#[cfg(test)]
fn is_memory_line(line: &str) -> bool {
    let code = line.split(" | ").next().unwrap_or_default();
    crate::model::Kind::ALL
        .iter()
        .any(|kind| kind.code() == code)
}

fn render(memory: &Memory, now: i64, addressing: Addressing) -> String {
    let statement = clip(&memory.content, LINE_CHARS);
    let subject = if memory.subject.is_empty() {
        "-"
    } else {
        memory.subject.as_str()
    };
    let mut line = format!(
        "{} | {} | {}: {}",
        memory.kind.code(),
        subject,
        age(memory.updated_at, now),
        statement.text
    );
    let id = match addressing {
        Addressing::Always => Some(memory.id.as_str()),
        Addressing::OnClip if statement.truncated => Some(memory.id.as_str()),
        Addressing::OnClip => None,
    };
    if let Some(id) = id {
        line.push(' ');
        line.push_str(id);
    }
    line
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
    // A canonical duplicate is one statement in two rows. Until the repair pass
    // closes it, the digest shows it once and says how many rows it hid.
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut hidden = 0usize;
    let mut shown: Vec<&Memory> = Vec::new();
    for memory in items {
        let key = (
            crate::store::fingerprint(&memory.content),
            memory.subject.clone(),
        );
        if seen.insert(key) {
            shown.push(memory);
        } else {
            hidden += 1;
        }
    }
    let mut lines: Vec<String> = shown.iter().map(|memory| line(memory, now)).collect();
    lines.extend(coverage_lines(&shown, coverage));
    if hidden > 0 {
        lines.push(format!(
            "duplicates: {hidden} hidden | repeated statements shown once"
        ));
    }
    lines.join("\n")
}

/// The coverage line, when the digest shows a subset.
fn coverage_lines(items: &[&Memory], coverage: &Coverage) -> Vec<String> {
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
            "more: {} | memory(recall, subject) for the rest",
            remaining.join(", ")
        ));
    }
    if !earlier.is_empty() {
        lines.push(format!(
            "earlier: {} | memory(recall, as_of) for what was true then",
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
    let lines: Vec<String> = rules.iter().map(|rule| rule_line(&rule.content)).collect();
    fill_rule_lines(&lines, max_tokens).1
}

/// Whether every rule, including one that is not stored yet, fits the render.
///
/// The write check and the prompt must measure the same block, or a write can
/// be admitted that the render then has to hide behind a notice.
pub fn rules_fit(existing: &[&Memory], content: &str, max_tokens: u64) -> bool {
    let mut lines: Vec<String> = existing
        .iter()
        .map(|rule| rule_line(&rule.content))
        .collect();
    lines.push(rule_line(content));
    fill_rule_lines(&lines, max_tokens).0 == lines.len()
}

/// The token cost of rendering every rule, with no notice.
pub fn rules_cost(existing: &[&Memory]) -> u64 {
    let text = existing
        .iter()
        .map(|rule| rule_line(&rule.content))
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

/// The token cost of a rule line that does not exist yet.
///
/// The write-time check and the render must measure the same line.
#[cfg(test)]
pub fn rule_cost_text(content: &str) -> u64 {
    hearth_model::estimate_tokens(&rule_line(content)).max(1) as u64
}

fn rule_line(content: &str) -> String {
    clip(content, RULE_CHARS).text
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
    fn a_line_is_a_delimited_header() {
        let line = line(&memory("m_1", "alice", "Alice manages the OR board."), 0);
        assert_eq!(line, "fact | alice | now: Alice manages the OR board.");
    }

    #[test]
    fn every_kind_is_the_first_field() {
        for (kind, code) in [
            (Kind::Rule, "rule"),
            (Kind::Preference, "pref"),
            (Kind::Person, "person"),
            (Kind::Decision, "decision"),
            (Kind::Commitment, "commit"),
            (Kind::Episode, "episode"),
            (Kind::Fact, "fact"),
        ] {
            let mut item = memory("m_1", "alice", "a statement");
            item.kind = kind;
            let line = line(&item, 0);
            assert!(line.starts_with(&format!("{code} | ")), "{line}");
        }
    }

    #[test]
    fn an_empty_subject_renders_a_dash() {
        let line = line(&memory("m_1", "", "the deploy pipeline has no owner"), 0);
        assert_eq!(line, "fact | - | now: the deploy pipeline has no owner");
    }

    #[test]
    fn a_multi_word_subject_is_one_field() {
        let line = line(
            &memory("m_1", "or board", "the OR board is reviewed on Mondays"),
            0,
        );
        assert_eq!(
            line,
            "fact | or board | now: the OR board is reviewed on Mondays"
        );
    }

    #[test]
    fn a_pipe_and_a_colon_inside_the_statement_keep_one_row() {
        let line = line(&memory("m_1", "topic", "a | b: c"), 0);
        assert_eq!(line, "fact | topic | now: a | b: c");
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn a_whole_row_carries_no_id() {
        let line = line(&memory("m_1", "alice", "a short statement"), 0);
        assert!(!line.contains("m_1"), "{line}");
    }

    #[test]
    fn a_clipped_row_ends_with_its_id() {
        let content = "b".repeat(crate::store::CONTENT_LIMIT + 10);
        let line = line(&memory("m_legacy", "topic", &content), 0);
        assert!(line.contains('…'), "{line}");
        assert!(line.ends_with("m_legacy"), "{line}");
    }

    #[test]
    fn a_tool_line_always_ends_with_its_id() {
        let line = tool_line(&memory("m_1", "alice", "a short statement"), 0);
        assert!(line.contains(" | "), "{line}");
        assert!(line.ends_with("m_1"), "{line}");
    }

    #[test]
    fn only_a_memory_row_starts_with_a_kind() {
        assert!(is_memory_line("fact | alice | now: a statement"));
        assert!(!is_memory_line(
            "more: alice 3 | memory(recall, subject) for the rest"
        ));
        assert!(!is_memory_line(
            "earlier: alice 2 | memory(recall, as_of) for what was true then"
        ));
        assert!(!is_memory_line("2 more rules were not shown."));
    }

    #[test]
    fn a_memory_at_the_content_limit_is_shown_in_full() {
        let content = "a".repeat(crate::store::CONTENT_LIMIT);
        let line = line(&memory("m_1", "topic", &content), 0);
        assert!(line.ends_with(content.as_str()), "{line}");
        assert!(!line.contains('…'), "a new memory is not clipped: {line}");
        assert!(!line.contains("m_1"), "a whole row carries no id: {line}");
    }

    #[test]
    fn an_older_row_above_the_content_limit_is_clipped() {
        let content = "b".repeat(crate::store::CONTENT_LIMIT + 10);
        let line = line(&memory("m_1", "topic", &content), 0);
        assert!(line.contains('…'), "an old row is clipped: {line}");
        assert!(!line.contains(content.as_str()), "{line}");
    }

    #[test]
    fn a_repeated_statement_is_shown_once_and_counted() {
        let items = vec![
            memory("m_1", "user", "the user prefers dark mode"),
            memory("m_2", "user", "the user prefers dark mode"),
        ];
        let block = recall_body(&items, &Coverage::default(), 0);
        assert_eq!(
            block.matches("the user prefers dark mode").count(),
            1,
            "the same statement must not spend tokens twice: {block}"
        );
        assert!(
            block.contains("1 hidden"),
            "the digest must say how many rows it collapsed: {block}"
        );
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
    fn the_rules_block_is_plain_lines() {
        let rules = vec![
            memory("m_1", "", "Never deploy on Friday"),
            memory("m_2", "", "Keep replies short"),
        ];
        let block = rules_body(&rules, 800);
        assert_eq!(block, "Never deploy on Friday\nKeep replies short");
        assert!(!block.contains("m_1"), "{block}");
        assert!(!block.contains(" | "), "{block}");
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

    #[test]
    fn the_coverage_hint_names_the_memory_tool() {
        let items = vec![memory("m_1", "alice", "one")];
        let coverage = Coverage {
            current: [("alice".to_owned(), 7)].into(),
            earlier: BTreeMap::new(),
        };
        let block = recall_body(&items, &coverage, 0);
        assert!(block.contains("memory(recall, subject)"), "{block}");
        assert!(!block.contains("memory_recall"), "{block}");
    }
}
