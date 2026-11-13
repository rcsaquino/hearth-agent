//! Model-facing rendering, ported from `src/results.ts`.
//!
//! The original returns a structured object and a human-readable rendering of it.
//! Hearth's tool contract is a single string, so these renderers produce that
//! string from the same fields, keeping the original's habit of stating plainly
//! what happened rather than making the model infer it.
//!
//! Two habits are preserved on purpose. A reused write says so, because a caller
//! that assumes its new tags were applied would be wrong. A refusal says the live
//! file was unchanged, because otherwise the safe response is to retry, which is
//! the wrong move.

use serde::Serialize;
use serde_json::json;

use crate::database::{Memory, MemoryRead, SearchResponse};
use crate::history::{HistoryRead, HistoryResponse};
use crate::standing::{
    StandingAddResult, StandingDeleteResult, StandingEditResult, StandingSnapshot,
};
use crate::text::STANDING_LIMIT;

/// Serialize a value for a tool result.
pub fn render<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|error| {
        json!({ "error": { "code": "render", "message": error.to_string() } }).to_string()
    })
}

/// The shared shape of a standing-store result.
fn standing_envelope(snapshot: &StandingSnapshot) -> serde_json::Value {
    json!({
        "cap": STANDING_LIMIT,
        "store": "standing",
        "characters": snapshot.characters,
        "remaining": snapshot.remaining,
        "entries": snapshot.entries.iter().map(|entry| json!({
            "id": entry.id, "priority": entry.priority, "content": entry.content
        })).collect::<Vec<_>>(),
        "archived": snapshot.archived.iter().map(|entry| json!({
            "id": entry.id, "priority": entry.priority, "content": entry.content
        })).collect::<Vec<_>>(),
        "text": snapshot.text,
    })
}

/// Render a standing add.
pub fn render_standing_add(operation: &str, result: &StandingAddResult) -> String {
    let mut value = standing_envelope(&result.snapshot);
    value["operation"] = json!(operation);
    value["memory"] = json!({
        "id": result.memory.id,
        "priority": result.memory.priority,
        "content": result.memory.content,
    });
    value["created"] = json!(result.created);
    let note = if !result.snapshot.archived.is_empty() {
        Some("Displaced bullets were saved in SQLite with tag standing-archive.".to_owned())
    } else {
        result.note.clone()
    };
    if let Some(note) = note {
        value["note"] = json!(note);
    }
    render(&value)
}

/// Render a standing edit.
pub fn render_standing_edit(operation: &str, result: &StandingEditResult) -> String {
    let mut value = standing_envelope(&result.snapshot);
    value["operation"] = json!(operation);
    value["memory"] = json!({
        "id": result.memory.id,
        "priority": result.memory.priority,
        "content": result.memory.content,
    });
    if !result.snapshot.archived.is_empty() {
        value["note"] = json!("Displaced bullets were saved in SQLite with tag standing-archive.");
    }
    render(&value)
}

/// Render a standing delete.
pub fn render_standing_delete(result: &StandingDeleteResult) -> String {
    let mut value = standing_envelope(&result.snapshot);
    value["deleted"] = json!({
        "id": result.deleted.id,
        "priority": result.deleted.priority,
        "content": result.deleted.content,
    });
    render(&value)
}

/// Render a long-term add or edit.
pub fn render_memory_write(operation: &str, memory: &Memory, created: Option<bool>) -> String {
    let mut value = json!({
        "store": "long_term",
        "operation": operation,
        "memory": memory,
    });
    if let Some(created) = created {
        value["created"] = json!(created);
        if !created {
            value["note"] = json!(
                "Identical content is already stored. Returned the existing memory without changing its tags, source, timestamps, or revision. Use memory_edit with its ID if information needs updating."
            );
        }
    }
    render(&value)
}

/// Render a long-term delete.
pub fn render_memory_delete(deleted: &Memory) -> String {
    render(&json!({
        "store": "long_term",
        "operation": "delete",
        "deleted": deleted,
    }))
}

/// Render a search response.
pub fn render_search(response: &SearchResponse) -> String {
    render(&json!({
        "mode": response.mode,
        "query": response.query,
        "tags": response.tags,
        "count": response.results.len(),
        "results": response.results,
        "next_offset": response.next_offset,
        "elapsed_ms": response.elapsed_ms,
        "hint": response.hint,
    }))
}

/// Render one memory read back in full.
pub fn render_memory_read(read: &MemoryRead) -> String {
    render(&json!({
        "store": "long_term",
        "memory": read.memory,
        "content": read.content,
        "next_text_offset": read.next_text_offset,
        "total_characters": read.total_characters,
    }))
}

/// Render a session search.
pub fn render_history_search(response: &HistoryResponse) -> String {
    render(&json!({
        "action": "search",
        "count": response.results.len(),
        "results": response.results,
        "next_offset": response.next_offset,
        "exhausted": response.exhausted,
        "candidate_entries": response.candidate_entries,
        "warnings": response.warnings,
        "elapsed_ms": response.elapsed_ms,
        "hint": response.hint,
    }))
}

/// Render one session entry.
pub fn render_history_read(read: &HistoryRead) -> String {
    render(&json!({
        "action": "read",
        "entry_id": read.entry_id,
        "conversation_id": read.conversation_id,
        "seq": read.seq,
        "role": read.role,
        "timestamp": read.timestamp,
        "citation": read.citation,
        "text": read.text,
        "next_text_offset": read.next_text_offset,
        "total_characters": read.total_characters,
        "next_line": read.next_line,
        "hint": read.hint,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reused_long_term_write_says_so() {
        let memory = Memory {
            id: "m_1".to_owned(),
            content: "fact".to_owned(),
            tags: vec![],
            source: String::new(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            revision: 1,
        };
        let rendered = render_memory_write("memory_add", &memory, Some(false));
        assert!(rendered.contains("already stored"));
        assert!(rendered.contains("\"created\": false"));
    }

    #[test]
    fn a_fresh_long_term_write_does_not_claim_reuse() {
        let memory = Memory {
            id: "m_1".to_owned(),
            content: "fact".to_owned(),
            tags: vec![],
            source: String::new(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            revision: 1,
        };
        let rendered = render_memory_write("memory_add", &memory, Some(true));
        assert!(!rendered.contains("already stored"));
    }
}
