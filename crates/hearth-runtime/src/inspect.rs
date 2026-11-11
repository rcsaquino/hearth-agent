//! Read-only views of the durable record.

use std::path::Path;

use serde_json::Value;
use thiserror::Error;

/// A read-only inspection failure.
#[derive(Debug, Error)]
pub enum InspectError {
    #[error(transparent)]
    Storage(#[from] hearth_storage::StorageError),
    #[error("database query failed: {0}")]
    Sqlite(#[from] hearth_storage::rusqlite::Error),
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

/// One saved memory row for the operator surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub kind: String,
    pub content: String,
    pub tags: Vec<String>,
    pub importance: i64,
    pub status: String,
    pub source_kind: String,
    pub source_ref: Option<String>,
    pub standing: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// Read saved memories without touching the database writer.
pub fn list_memories(home: &Path, limit: usize) -> Result<Vec<MemoryRecord>, InspectError> {
    let database = home.join("hearth.db");
    if !database.is_file() {
        return Ok(Vec::new());
    }
    let connection = hearth_storage::open_read_only(&database)?;
    let mut statement = connection.prepare(
        "SELECT id, kind, content, tags, importance, status, source_kind, source_ref, \
         standing, created_at, last_used_at FROM memory_items \
         ORDER BY created_at DESC, rowid DESC LIMIT ?",
    )?;
    let rows = statement.query_map((limit as i64,), |row| {
        let tags: String = row.get(3)?;
        Ok(MemoryRecord {
            id: row.get(0)?,
            kind: row.get(1)?,
            content: row.get(2)?,
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            importance: row.get(4)?,
            status: row.get(5)?,
            source_kind: row.get(6)?,
            source_ref: row.get(7)?,
            standing: row.get(8)?,
            created_at: row.get(9)?,
            last_used_at: row.get(10)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}
