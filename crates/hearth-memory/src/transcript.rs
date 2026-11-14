//! The evidence store: what was actually said.
//!
//! The transcript is deliberately not memory. It is never edited, never
//! summarised in place, and never rewritten by a correction. It is where a
//! memory's provenance points and where an answer about a specific past
//! conversation comes from.
//!
//! Search runs on `entries_fts`, which indexes the stored JSON. The tokenizer
//! ignores punctuation, so ordinary words match, and the reference
//! `conversation#seq` is stable enough to cite.

use hearth_core::Session;
use serde_json::Value;

use crate::errors::{Code, MemoryError, Result};
use crate::query::analyze;

/// Longest excerpt a search result shows.
const SNIPPET_CHARS: usize = 240;
/// Longest entry text a read returns in one page.
const READ_CHARS: usize = 20_000;

/// One transcript hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryHit {
    pub conversation_id: String,
    pub seq: i64,
    pub kind: String,
    pub snippet: String,
    pub created_at: i64,
}

impl EntryHit {
    /// The citation a caller can pass back to `memory_transcript`.
    pub fn reference(&self) -> String {
        format!("{}#{}", self.conversation_id, self.seq)
    }
}

/// One transcript entry, read in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRead {
    pub conversation_id: String,
    pub seq: i64,
    pub kind: String,
    pub text: String,
    pub created_at: i64,
}

/// Search the transcript.
pub fn search(session: &Session, text: &str, limit: i64) -> Result<Vec<EntryHit>> {
    let known = Default::default();
    let parsed = analyze(text, &known);
    let expression = crate::retrieval::match_any(&parsed.terms, &parsed.expanded, &parsed.entities)
        .or_else(|| crate::retrieval::match_all(&parsed.terms));
    let Some(expression) = expression else {
        return Ok(Vec::new());
    };
    session
        .storage()
        .read(|reader| {
            reader.query(
                "SELECT e.conversation_id, e.seq, e.kind, e.content_json, e.created_at \
                 FROM entries_fts f JOIN entries e ON e.rowid = f.rowid \
                 WHERE entries_fts MATCH ? ORDER BY bm25(entries_fts) LIMIT ?",
                rusqlite::params![expression, limit.clamp(1, 50)],
                |row| {
                    let json: String = row.get(3)?;
                    Ok(EntryHit {
                        conversation_id: row.get(0)?,
                        seq: row.get(1)?,
                        kind: row.get(2)?,
                        snippet: snippet(&decode(&json)),
                        created_at: row.get(4)?,
                    })
                },
            )
        })
        .map_err(Into::into)
}

/// Read one entry by `conversation#seq`.
pub fn read(session: &Session, reference: &str) -> Result<Option<TranscriptRead>> {
    let Some((conversation_id, seq)) = reference.trim().rsplit_once('#') else {
        return Err(MemoryError::new(
            Code::InvalidInput,
            "a transcript reference looks like conversation#seq.",
        ));
    };
    let seq: i64 = seq.trim().parse().map_err(|_| {
        MemoryError::new(
            Code::InvalidInput,
            "a transcript reference looks like conversation#seq.",
        )
    })?;
    let conversation_id = conversation_id.trim().to_owned();
    session
        .storage()
        .read(|reader| {
            reader.query_optional(
                "SELECT kind, content_json, created_at FROM entries \
                 WHERE conversation_id = ? AND seq = ?",
                rusqlite::params![conversation_id, seq],
                |row| {
                    let json: String = row.get(1)?;
                    let mut text = decode(&json);
                    if text.chars().count() > READ_CHARS {
                        text = text.chars().take(READ_CHARS).collect();
                        text.push('…');
                    }
                    Ok(TranscriptRead {
                        conversation_id: conversation_id.clone(),
                        seq,
                        kind: row.get(0)?,
                        text,
                        created_at: row.get(2)?,
                    })
                },
            )
        })
        .map_err(Into::into)
}

/// Flatten a stored entry's blocks into plain text.
fn decode(json: &str) -> String {
    let Ok(value) = serde_json::from_str::<Vec<Value>>(json) else {
        return json.to_owned();
    };
    let mut text = String::new();
    for block in value {
        let Some(kind) = block.get("type").and_then(Value::as_str) else {
            continue;
        };
        let body = match kind {
            "text" => block.get("text").and_then(Value::as_str),
            "thinking" => block.get("thinking").and_then(Value::as_str),
            "tool_call" => block.get("name").and_then(Value::as_str),
            "tool_result" => block.get("content").and_then(Value::as_str),
            _ => None,
        };
        if let Some(body) = body {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(body);
        }
    }
    if text.is_empty() {
        json.to_owned()
    } else {
        text
    }
}

fn snippet(text: &str) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut clipped: String = flattened.chars().take(SNIPPET_CHARS).collect();
    if flattened.chars().count() > SNIPPET_CHARS {
        clipped.push('…');
    }
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearth_core::{Harness, Save};
    use tempfile::TempDir;

    fn with_entry(text: &str) -> (TempDir, Session, String) {
        let directory = TempDir::new().expect("temp home");
        let harness = Harness::open(directory.path()).expect("harness");
        let session = harness.session().clone();
        let conversation = session
            .create_conversation("Test", "local", "local:1")
            .expect("conversation");
        session
            .save(Save::new(&conversation.id).entry("user", serde_json::json!(text)))
            .expect("save");
        (directory, session, conversation.id)
    }

    #[test]
    fn a_saved_message_is_findable_and_citable() {
        let (_directory, session, conversation) =
            with_entry("we agreed the OR board meets on Thursdays");
        let hits = search(&session, "when does the OR board meet", 5).expect("search");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("Thursdays"), "{:?}", hits[0]);
        assert_eq!(hits[0].reference(), format!("{conversation}#1"));
        let read = read(&session, &hits[0].reference())
            .expect("read")
            .expect("entry");
        assert!(read.text.contains("Thursdays"));
    }

    #[test]
    fn a_malformed_reference_is_refused() {
        let (_directory, session, _conversation) = with_entry("anything");
        assert_eq!(
            read(&session, "nonsense").unwrap_err().code,
            Code::InvalidInput
        );
    }
}
