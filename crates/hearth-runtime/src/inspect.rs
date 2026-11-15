//! Read-only views of the durable record.

use std::path::Path;

use hearth_core::Zone;
use serde_json::{Value, json};
use thiserror::Error;

/// A read-only inspection failure.
#[derive(Debug, Error)]
pub enum InspectError {
    #[error(transparent)]
    Storage(#[from] hearth_storage::StorageError),
    #[error("database query failed: {0}")]
    Sqlite(#[from] hearth_storage::rusqlite::Error),
    #[error("memory could not be read: {0}")]
    Memory(String),
}

/// Read the newest saved prompt document without touching the database writer.
pub fn latest_prompt(home: &Path) -> Result<Option<Value>, InspectError> {
    let database = home.join("hearth.db");
    if !database.is_file() {
        return Ok(None);
    }
    let connection = hearth_storage::open_read_only(&database)?;
    let mut statement = connection
        .prepare("SELECT text FROM prompt_blobs ORDER BY created_at DESC, rowid DESC LIMIT 50")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for text in rows {
        let text = text?;
        if !text.trim_start().starts_with('{') {
            continue;
        }
        if let Ok(document) = serde_json::from_str(&text) {
            return Ok(Some(document));
        }
    }
    Ok(None)
}

/// Newest-first prompt/response exchanges for the Prompt audit tab.
///
/// Each item carries the saved request document, the hash it was stored under,
/// and the assistant entry that answered it. `usage` is always null today: the
/// schema has no `prompt_hash` on `usage_events`, and the only candidate link
/// (`usage_events.task_id` -> `tasks.checkpoint_json.prompt_hash`) is rewritten
/// every turn, so a task with many turns keeps just its latest hash. Attaching
/// usage by task or timestamp would be a guess.
pub fn prompt_exchanges(home: &Path, limit: usize) -> Result<Vec<Value>, InspectError> {
    let database = home.join("hearth.db");
    if !database.is_file() {
        return Ok(Vec::new());
    }
    let connection = hearth_storage::open_read_only(&database)?;
    let want = limit.max(1);
    let mut statement = connection.prepare(
        "SELECT b.hash, b.created_at, b.text, e.id, e.created_at, e.content_json, e.tokens_est \
         FROM prompt_blobs b LEFT JOIN entries e ON e.prompt_hash = b.hash \
         ORDER BY b.created_at DESC, b.rowid DESC LIMIT ?",
    )?;
    // Over-fetch: non-JSON blobs are skipped, exactly like `latest_prompt`.
    let rows = statement.query_map((want.saturating_mul(2) as i64,), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<i64>>(6)?,
        ))
    })?;
    let mut exchanges: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for row in rows {
        let (hash, created_at, text, entry_id, entry_at, content, tokens) = row?;
        if exchanges.len() >= want {
            break;
        }
        if !text.trim_start().starts_with('{') {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        // A hash answers at most one entry; ignore duplicates from the join.
        let duplicate = seen.iter().any(|seen| seen == &hash);
        if duplicate {
            continue;
        }
        seen.push(hash.clone());
        let response = match (entry_id, entry_at, content) {
            (Some(entry_id), Some(entry_at), Some(content)) => {
                let blocks = serde_json::from_str::<Value>(&content)
                    .ok()
                    .filter(Value::is_array)
                    .unwrap_or(Value::Array(Vec::new()));
                json!({
                    "entry_id": entry_id,
                    "created_at": entry_at,
                    "blocks": blocks,
                    "tokens_est": tokens.unwrap_or(0),
                })
            }
            _ => Value::Null,
        };
        exchanges.push(json!({
            "request": request,
            "prompt_hash": hash,
            "requested_at": created_at,
            "response": response,
            "usage": Value::Null,
        }));
    }
    Ok(exchanges)
}

/// One saved memory row for the operator surface.
///
/// One saved statement, as the operator surface shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub content: String,
    pub kind: String,
    pub subject: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: i64,
    /// True when this row is a rule injected into every request.
    pub pinned: bool,
}

/// Read saved memories from the main database without touching the writer.
///
/// A read-only connection keeps the operator surface from competing with the
/// daemon for the write lock.
pub fn list_memories(home: &Path, limit: usize) -> Result<Vec<MemoryRecord>, InspectError> {
    let database = home.join("hearth.db");
    if !database.is_file() {
        return Ok(Vec::new());
    }
    let connection = hearth_storage::open_read_only(&database)?;
    let zone = Zone::resolve(&hearth_config::identity_timezone(home));
    let mut statement = connection.prepare(
        "SELECT id, content, kind, subject, source_kind, created_at, updated_at, revision, pinned \
         FROM memories ORDER BY pinned DESC, updated_at DESC, rowid DESC LIMIT ?",
    )?;
    let rows = statement.query_map((limit as i64,), |row| {
        Ok(MemoryRecord {
            id: row.get(0)?,
            content: row.get(1)?,
            kind: row.get(2)?,
            subject: row.get(3)?,
            source: row.get(4)?,
            created_at: zone.iso(row.get(5)?),
            updated_at: zone.iso(row.get(6)?),
            revision: row.get(7)?,
            pinned: row.get::<_, i64>(8)? != 0,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}
