//! Read-only views of the durable record.

use std::path::Path;

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
/// Standing bullets and long-term memories are shown in one list because that is
/// how they are experienced: both are things the agent knows. The `standing` flag
/// says which file the row came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    /// Provenance for a long-term memory, or the priority for a standing bullet.
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: i64,
    /// True when this row is a bullet from `standing.md`.
    pub standing: bool,
}

/// Read saved memories without touching the database writer.
///
/// Long-term memories come from the memory database, standing bullets from the
/// standing file. A missing file is an empty list, not an error: a fresh install
/// has nothing saved yet.
pub fn list_memories(home: &Path, limit: usize) -> Result<Vec<MemoryRecord>, InspectError> {
    let mut records = Vec::new();
    let database = home.join("memory").join("long-term.sqlite");
    if database.is_file() {
        let connection = hearth_storage::open_read_only(&database)?;
        let mut statement = connection.prepare(
            "SELECT id, content, tags, source, created_at, updated_at, revision \
             FROM memories ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )?;
        let rows = statement.query_map((limit as i64,), |row| {
            let tags: String = row.get(2)?;
            Ok(MemoryRecord {
                id: row.get(0)?,
                content: row.get(1)?,
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                source: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                revision: row.get(6)?,
                standing: false,
            })
        })?;
        for row in rows {
            records.push(row?);
        }
    }

    let standing_path = home.join("memory").join("standing.md");
    if standing_path.is_file() {
        let archive: std::sync::Arc<hearth_memory::ArchiveFn> = std::sync::Arc::new(
            |_: &[hearth_memory::StandingItem]| -> hearth_memory::Result<()> { Ok(()) },
        );
        let store = hearth_memory::StandingStore::new(&standing_path, archive);
        let items = store
            .entries()
            .map_err(|error| InspectError::Memory(error.to_string()))?;
        for item in items {
            records.push(MemoryRecord {
                id: item.id,
                content: item.content,
                tags: Vec::new(),
                source: format!("standing.md, priority {}", item.priority),
                created_at: String::new(),
                updated_at: String::new(),
                revision: 0,
                standing: true,
            });
        }
    }
    Ok(records)
}
