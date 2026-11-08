//! Compaction planning and the summary entry contract.

use serde_json::{Value, json};

use crate::model::Entry;

/// A range that will be replaced by one summary entry.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionPlan {
    /// The entries covered by the new summary, oldest first.
    pub entries: Vec<Entry>,
    /// The first covered sequence number.
    pub first_seq: i64,
    /// The last covered sequence number.
    pub last_seq: i64,
    /// The first sequence number kept after the summary.
    pub first_kept: i64,
}

/// Choose the oldest range to summarize, keeping at least `keep_recent_tokens`
/// at the tail and cutting at a user or assistant entry.
pub fn plan(history: &[Entry], keep_recent_tokens: u64) -> Option<CompactionPlan> {
    if history.len() < 2 {
        return None;
    }
    let keep = keep_recent_tokens.max(1);
    let mut index = history.len();
    let mut kept_tokens = 0u64;
    while index > 0 {
        index -= 1;
        kept_tokens = kept_tokens.saturating_add(entry_tokens(&history[index]));
        if kept_tokens >= keep {
            break;
        }
    }
    while index > 0 && !matches!(history[index].kind.as_str(), "user" | "assistant") {
        index -= 1;
    }
    if index == 0 {
        return None;
    }
    let covered = &history[..index];
    let first_seq = covered.first().map(|entry| {
        summary_covers(entry)
            .map(|(first, _)| first)
            .unwrap_or(entry.seq)
    })?;
    let last_seq = covered.last().map(|entry| {
        summary_covers(entry)
            .map(|(_, last)| last)
            .unwrap_or(entry.seq)
    })?;
    let first_kept = history[index].seq;
    Some(CompactionPlan {
        entries: covered.to_vec(),
        first_seq,
        last_seq,
        first_kept,
    })
}

fn entry_tokens(entry: &Entry) -> u64 {
    if entry.tokens_est > 0 {
        entry.tokens_est as u64
    } else {
        crate::prompt::estimate_content(&entry.content).max(1) as u64
    }
}

fn summary_covers(entry: &Entry) -> Option<(i64, i64)> {
    if entry.kind != "summary" {
        return None;
    }
    let covers = entry.content.get("covers")?;
    Some((
        covers.get("first_seq")?.as_i64()?,
        covers.get("last_seq")?.as_i64()?,
    ))
}

/// Build the JSON stored on a summary entry.
pub fn summary_content(text: &str, plan: &CompactionPlan) -> Value {
    json!({
        "text": text,
        "covers": {"first_seq": plan.first_seq, "last_seq": plan.last_seq},
        "first_kept": plan.first_kept,
    })
}

/// The summary text of a summary entry.
pub fn summary_text(entry: &Entry) -> Option<&str> {
    if entry.kind != "summary" {
        return None;
    }
    entry.content.get("text").and_then(Value::as_str)
}

/// The first kept sequence recorded on a summary entry.
pub fn summary_first_kept(entry: &Entry) -> Option<i64> {
    if entry.kind != "summary" {
        return None;
    }
    entry.content.get("first_kept").and_then(Value::as_i64)
}
