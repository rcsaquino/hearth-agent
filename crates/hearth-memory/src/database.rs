//! The SQLite long-term store, ported from `src/database.ts`.
//!
//! The shape is the original's: a `memories` table with JSON `tags` plus a
//! flattened `tag_text` mirror for the index, an FTS5 table kept in sync by
//! triggers, and a literal-substring fallback when the index cannot answer.
//!
//! Two behaviors are worth calling out because they are easy to lose in a port:
//!
//! * **`add` is idempotent on content.** An identical memory returns the existing
//!   row with `created = false` and does not touch its tags, source, timestamps,
//!   or revision. The original explains this in a note so a caller does not
//!   assume the new tags were applied.
//! * **An empty FTS result falls back to a substring scan.** The index cannot
//!   match a term it never tokenized, and reporting "not found" for a phrase that
//!   is plainly in the text would be a lie. The fallback is decided before
//!   pagination is applied so page two means the same thing as page one.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ExpectedError, Result};
use crate::schema;
use crate::text::{
    MEMORY_LIMIT, NEW_MEMORY_LIMIT, chunk, excerpt, integer, no_credentials, nonempty, tags,
};

/// One long-term memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: i64,
}

/// Input for a new memory.
#[derive(Debug, Clone, Default)]
pub struct MemoryInput {
    pub content: String,
    pub tags: Option<Vec<String>>,
    pub source: Option<String>,
}

/// How a query is matched against the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Any keyword prefix matches.
    Any,
    /// Every keyword must match.
    All,
    /// The keywords must appear as a token phrase.
    Phrase,
    /// A case-insensitive substring match, bypassing the index.
    Literal,
}

impl SearchMode {
    /// Parse the wire spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "any" => Some(Self::Any),
            "all" => Some(Self::All),
            "phrase" => Some(Self::Phrase),
            "literal" => Some(Self::Literal),
            _ => None,
        }
    }
}

/// Extra filters for a search.
#[derive(Debug, Clone, Default)]
pub struct MemorySearchOptions {
    /// Every supplied tag must be present, compared case-sensitively.
    pub tags: Vec<String>,
}

/// One search result: a memory plus how it will be shown.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    #[serde(flatten)]
    pub memory: Memory,
    /// An excerpt centered on the first term.
    pub content: String,
    pub content_characters: usize,
    pub content_clipped: bool,
}

/// The response to a search.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    /// The mode actually used, which may differ from the requested one.
    pub mode: String,
    pub query: String,
    pub tags: Vec<String>,
    pub results: Vec<SearchHit>,
    pub next_offset: Option<i64>,
    pub elapsed_ms: f64,
    pub hint: String,
}

/// A chunk of a memory's full content.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryRead {
    #[serde(flatten)]
    pub memory: Memory,
    pub content: String,
    pub next_text_offset: Option<usize>,
    pub total_characters: usize,
}

/// The long-term memory database.
///
/// `rusqlite::Connection` is not `Sync`, so it is held behind a mutex. Each
/// operation takes the guard for its whole duration, which also means a
/// transaction is never interleaved with another caller's statement.
pub struct MemoryStore {
    connection: Mutex<Connection>,
    path: PathBuf,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl MemoryStore {
    /// Open the store, creating or verifying the schema.
    ///
    /// A fresh file is initialized and signed. An existing file must already be
    /// ours, which is checked before any write, so a path that accidentally
    /// points at another database is refused instead of migrated.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        // Folds both sides of the literal fallback so a case-insensitive
        // substring match works for scripts the tokenizer does not fold.
        connection.create_scalar_function(
            "hearth_fold",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                let value: String = context.get(0)?;
                Ok(value.to_lowercase())
            },
        )?;
        schema::prepare(&connection)
            .map_err(|message| ExpectedError::new(ErrorCode::StoreUnavailable, message))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self {
            connection: Mutex::new(connection),
            path,
        })
    }

    /// The file backing this store.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Take the connection for the duration of one operation.
    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| {
            ExpectedError::new(ErrorCode::StoreUnavailable, "memory database lock poisoned")
        })
    }

    /// Read one memory by id, using a connection the caller already holds.
    ///
    /// Every path that is already inside a transaction uses this, because taking
    /// the mutex twice in one thread would deadlock.
    fn get_inner(connection: &Connection, id: &str) -> Result<Memory> {
        let mut statement = connection.prepare("SELECT * FROM memories WHERE id=?")?;
        let row = statement
            .query_row([id], row_to_memory)
            .optional()?
            .ok_or_else(|| {
                ExpectedError::with_details(
                    ErrorCode::NotFound,
                    format!("Memory {id} does not exist. Search before editing or deleting."),
                    serde_json::json!({ "id": id }),
                )
            })?;
        Ok(row)
    }

    /// Read one memory by id.
    pub fn get(&self, id: &str) -> Result<Memory> {
        let connection = self.lock()?;
        Self::get_inner(&connection, id)
    }

    /// Read a memory's full content in a bounded window.
    pub fn read(&self, id: &str, text_offset: usize) -> Result<MemoryRead> {
        let connection = self.lock()?;
        let memory = Self::get_inner(&connection, id)?;
        let part = chunk(&memory.content, text_offset, 6_000);
        Ok(MemoryRead {
            memory,
            content: part.text,
            next_text_offset: part.next_text_offset,
            total_characters: part.total_characters,
        })
    }

    /// Save a memory, reusing an identical one.
    ///
    /// The lookup and the insert share one immediate transaction so two writers
    /// cannot both decide the content is new.
    pub fn add(&self, input: &MemoryInput) -> Result<(Memory, bool)> {
        let content = nonempty(&input.content, "content", NEW_MEMORY_LIMIT)?;
        no_credentials(&content)?;
        let tag_list = tags(input.tags.as_deref())?;
        let source = match input.source.as_deref() {
            Some(value) => nonempty(value, "source", 2_000)?,
            None => String::new(),
        };
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        {
            let mut statement = connection
                .prepare("SELECT * FROM memories WHERE content=? ORDER BY rowid LIMIT 1")?;
            if let Some(existing) = statement.query_row([&content], row_to_memory).optional()? {
                return Ok((existing, false));
            }
        }
        let id = format!("m_{}", uuid::Uuid::new_v4());
        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        connection.execute(
            "INSERT INTO memories(id,content,tags,tag_text,source,created_at,updated_at) VALUES(?,?,?,?,?,?,?)",
            rusqlite::params![
                &id,
                &content,
                serde_json::to_string(&tag_list).unwrap_or_else(|_| "[]".to_owned()),
                tag_list.join("\n"),
                &source,
                &timestamp,
                &timestamp,
            ],
        )?;
        let saved = Self::get_inner(&connection, &id)?;
        transaction.commit()?;
        Ok((saved, true))
    }

    /// Archive evicted standing bullets so nothing is lost.
    ///
    /// Each entry is keyed by its original id plus a hash of its content, so a
    /// retry is idempotent and a bullet evicted twice does not duplicate. Content
    /// over the memory limit is split into complete, bounded pieces.
    pub fn archive(
        &self,
        entries: &[crate::standing::StandingItem],
        source_path: &str,
    ) -> Result<Vec<String>> {
        use sha2::{Digest, Sha256};
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        let mut ids = Vec::new();
        for entry in entries {
            let mut hasher = Sha256::new();
            hasher.update(entry.id.as_bytes());
            hasher.update(b"\0");
            hasher.update(entry.content.as_bytes());
            let digest = format!("{:x}", hasher.finalize());
            let base = format!("m_archive_{}", &digest[..24]);
            let mut parts = Vec::new();
            let mut offset = 0;
            loop {
                let part = chunk(&entry.content, offset, 19_000);
                parts.push(part.text);
                match part.next_text_offset {
                    Some(next) => offset = next,
                    None => break,
                }
            }
            let total = parts.len();
            for (index, part) in parts.into_iter().enumerate() {
                let id = if total == 1 {
                    base.clone()
                } else {
                    format!("{base}_{}", index + 1)
                };
                let source = if total == 1 {
                    format!("{source_path}#{}", entry.id)
                } else {
                    format!("{source_path}#{} (part {}/{total})", entry.id, index + 1)
                };
                let content = nonempty(&part, "content", MEMORY_LIMIT)?;
                let tag_list = ["standing-archive".to_owned()];
                let timestamp = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default();
                connection.execute(
                    "INSERT OR IGNORE INTO memories(id,content,tags,tag_text,source,created_at,updated_at) VALUES(?,?,?,?,?,?,?)",
                    rusqlite::params![
                        &id,
                        &content,
                        serde_json::to_string(&tag_list).unwrap_or_else(|_| "[]".to_owned()),
                        tag_list.join("\n"),
                        &source,
                        &timestamp,
                        &timestamp,
                    ],
                )?;
            }
            ids.push(base);
        }
        transaction.commit()?;
        Ok(ids)
    }

    /// Update a memory, preserving fields that are not supplied.
    pub fn edit(
        &self,
        id: &str,
        content: Option<&str>,
        new_tags: Option<&[String]>,
        source: Option<&str>,
        expected_revision: Option<i64>,
    ) -> Result<Memory> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        let previous = Self::get_inner(&connection, id)?;
        check_revision(&previous, expected_revision)?;
        let content = match content {
            Some(value) => {
                let content = nonempty(value, "content", NEW_MEMORY_LIMIT)?;
                no_credentials(&content)?;
                content
            }
            None => previous.content.clone(),
        };
        let tag_list = match new_tags {
            Some(value) => tags(Some(value))?,
            None => previous.tags.clone(),
        };
        let source = match source {
            Some("") => String::new(),
            Some(value) => nonempty(value, "source", 2_000)?,
            None => previous.source.clone(),
        };
        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        connection.execute(
            "UPDATE memories SET content=?,tags=?,tag_text=?,source=?,updated_at=?,revision=revision+1 WHERE id=?",
            rusqlite::params![
                &content,
                serde_json::to_string(&tag_list).unwrap_or_else(|_| "[]".to_owned()),
                tag_list.join("\n"),
                &source,
                &timestamp,
                id,
            ],
        )?;
        let updated = Self::get_inner(&connection, id)?;
        transaction.commit()?;
        Ok(updated)
    }

    /// Delete a memory and return what was removed.
    pub fn delete(&self, id: &str, expected_revision: Option<i64>) -> Result<Memory> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        let previous = Self::get_inner(&connection, id)?;
        check_revision(&previous, expected_revision)?;
        connection.execute("DELETE FROM memories WHERE id=?", [id])?;
        transaction.commit()?;
        Ok(previous)
    }

    /// Search long-term memory.
    pub fn search(
        &self,
        query: &str,
        mode: SearchMode,
        limit: i64,
        offset: i64,
        options: &MemorySearchOptions,
    ) -> Result<SearchResponse> {
        let limit = integer(limit, "limit", 1, 50)?;
        let offset = integer(offset, "offset", 0, i64::MAX)?;
        if query.contains('\0') || query.chars().count() > 2_000 {
            return Err(ExpectedError::with_details(
                ErrorCode::InvalidInput,
                "Query must be at most 2,000 characters without NUL bytes.",
                serde_json::json!({ "field": "query" }),
            ));
        }
        let requested_tags = tags(Some(&options.tags))?;
        let started = std::time::Instant::now();
        let connection = self.lock()?;
        let trimmed = query.trim();
        let mut terms: Vec<String> = Vec::new();
        for term in split_terms(trimmed) {
            if !terms.contains(&term) {
                terms.push(term);
            }
        }

        // The tag predicate is applied inside SQL so it filters before pagination.
        let tag_clause = if requested_tags.is_empty() {
            String::new()
        } else {
            " AND ".to_owned()
                + &requested_tags
                    .iter()
                    .map(|_| {
                        "EXISTS (SELECT 1 FROM json_each(m.tags) AS requested_tag WHERE requested_tag.value = ? COLLATE BINARY)"
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ")
        };

        let mut used_mode = match mode {
            SearchMode::Any => "any",
            SearchMode::All => "all",
            SearchMode::Phrase => "phrase",
            SearchMode::Literal => "literal",
        };
        let mut rows: Vec<Memory>;
        let mut has_more = false;

        if trimmed.is_empty() {
            used_mode = "browse";
            let sql = format!(
                "SELECT m.* FROM memories m{} ORDER BY m.created_at,m.id LIMIT ? OFFSET ?",
                if tag_clause.is_empty() {
                    String::new()
                } else {
                    format!(" WHERE {}", tag_clause.trim_start_matches(" AND "))
                }
            );
            let mut values: Vec<rusqlite::types::Value> = requested_tags
                .iter()
                .map(|tag| rusqlite::types::Value::Text(tag.clone()))
                .collect();
            values.push(rusqlite::types::Value::Integer(limit + 1));
            values.push(rusqlite::types::Value::Integer(offset));
            rows = collect(&connection, &sql, values)?;
        } else if mode == SearchMode::Literal || terms.is_empty() {
            used_mode = "literal";
            rows = Self::literal(
                &connection,
                trimmed,
                &tag_clause,
                &requested_tags,
                limit,
                offset,
            )?;
        } else {
            let fts = match mode {
                SearchMode::Phrase => format!("\"{}\"", terms.join(" ")),
                _ => {
                    let joiner = if mode == SearchMode::All {
                        " AND "
                    } else {
                        " OR "
                    };
                    terms
                        .iter()
                        .map(|term| format!("\"{term}\"*"))
                        .collect::<Vec<_>>()
                        .join(joiner)
                }
            };
            let sql = format!(
                "SELECT m.* FROM memories_fts JOIN memories m ON m.rowid=memories_fts.rowid
                 WHERE memories_fts MATCH ?{tag_clause} ORDER BY bm25(memories_fts),m.id LIMIT ? OFFSET ?"
            );
            let mut values: Vec<rusqlite::types::Value> =
                vec![rusqlite::types::Value::Text(fts.clone())];
            values.extend(
                requested_tags
                    .iter()
                    .map(|tag| rusqlite::types::Value::Text(tag.clone())),
            );
            values.push(rusqlite::types::Value::Integer(limit + 1));
            values.push(rusqlite::types::Value::Integer(offset));
            rows = collect(&connection, &sql, values)?;

            // Decide the fallback independently of the page offset so a later
            // page keeps the same semantics as the first one. A corpus that is
            // entirely filtered out by tags also falls back, which is why the
            // probe repeats the tag predicate.
            let probe = format!(
                "SELECT m.rowid FROM memories_fts JOIN memories m ON m.rowid=memories_fts.rowid
                 WHERE memories_fts MATCH ?{tag_clause} LIMIT 1"
            );
            let mut probe_values: Vec<rusqlite::types::Value> =
                vec![rusqlite::types::Value::Text(fts)];
            probe_values.extend(
                requested_tags
                    .iter()
                    .map(|tag| rusqlite::types::Value::Text(tag.clone())),
            );
            let any: Option<i64> = connection
                .prepare(&probe)?
                .query_row(params_from_iter(probe_values), |row| row.get(0))
                .optional()?;
            if any.is_none() {
                used_mode = "literal";
                rows = Self::literal(
                    &connection,
                    trimmed,
                    &tag_clause,
                    &requested_tags,
                    limit,
                    offset,
                )?;
            }
        }

        if rows.len() as i64 > limit {
            has_more = true;
            rows.truncate(limit as usize);
        }
        let first_term = terms.first().cloned().unwrap_or_default();
        let results: Vec<SearchHit> = rows
            .into_iter()
            .map(|memory| {
                let characters = memory.content.chars().count();
                SearchHit {
                    content: excerpt(&memory.content, &first_term, 600),
                    content_characters: characters,
                    content_clipped: characters > 600,
                    memory,
                }
            })
            .collect();

        Ok(SearchResponse {
            mode: used_mode.to_owned(),
            query: trimmed.to_owned(),
            tags: requested_tags,
            results,
            next_offset: if has_more { Some(offset + limit) } else { None },
            elapsed_ms: started.elapsed().as_secs_f64() * 1_000.0,
            hint: "Use an m_ id to read a complete SQLite memory; standing s_ entries are in the injected standing.md. Try mode=literal for exact substrings, shorter/alternate terms, or query='' to browse SQLite. For historical claims or uncertain/empty recall, use memory_sessions; an empty keyword result does not prove absence.".to_owned(),
        })
    }

    /// The substring fallback.
    ///
    /// The original folds both sides with a registered function so the match is
    /// case-insensitive for scripts the tokenizer does not fold. The same
    /// function is registered here.
    fn literal(
        connection: &Connection,
        needle: &str,
        tag_clause: &str,
        requested_tags: &[String],
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Memory>> {
        let sql = format!(
            "SELECT m.* FROM memories m
             WHERE instr(hearth_fold(m.content || char(10) || m.tag_text || char(10) || m.source), hearth_fold(?)) > 0{tag_clause}
             ORDER BY m.created_at, m.id LIMIT ? OFFSET ?"
        );
        let mut values: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Text(needle.to_owned())];
        values.extend(
            requested_tags
                .iter()
                .map(|tag| rusqlite::types::Value::Text(tag.clone())),
        );
        values.push(rusqlite::types::Value::Integer(limit + 1));
        values.push(rusqlite::types::Value::Integer(offset));
        collect(connection, &sql, values)
    }

    /// Count memories, for diagnostics.
    pub fn count(&self) -> Result<i64> {
        let connection = self.lock()?;
        Ok(connection.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?)
    }
}

/// Reject a stale write using the optimistic revision guard.
fn check_revision(previous: &Memory, expected_revision: Option<i64>) -> Result<()> {
    let Some(expected) = expected_revision else {
        return Ok(());
    };
    if previous.revision == expected {
        return Ok(());
    }
    Err(ExpectedError::with_details(
        ErrorCode::Conflict,
        format!(
            "Memory {} changed (revision {}, expected {expected}). Search again before changing it.",
            previous.id, previous.revision
        ),
        serde_json::json!({
            "id": previous.id,
            "current_revision": previous.revision,
            "expected_revision": expected,
        }),
    ))
}

/// Split a query into index terms.
///
/// The original uses a Unicode identifier character class. `char::is_alphanumeric`
/// plus combining marks is the closest portable equivalent.
fn split_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' || is_combining_mark(character) {
            current.push(character);
        } else if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    terms
}

/// True for the Unicode combining-mark ranges common in decomposed text.
fn is_combining_mark(character: char) -> bool {
    matches!(character as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF)
}

/// Map a row to a memory.
fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let tags: String = row.get("tags")?;
    Ok(Memory {
        id: row.get("id")?,
        content: row.get("content")?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        source: row.get("source")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        revision: row.get("revision")?,
    })
}

/// Run a query that selects whole memory rows.
fn collect(
    connection: &Connection,
    sql: &str,
    values: Vec<rusqlite::types::Value>,
) -> Result<Vec<Memory>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement
        .query_map(params_from_iter(values), row_to_memory)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::standing::StandingItem;

    fn database() -> (tempfile::TempDir, MemoryStore) {
        let directory = tempfile::tempdir().unwrap();
        let database = MemoryStore::open(directory.path().join("long-term.sqlite")).unwrap();
        (directory, database)
    }

    fn save(database: &MemoryStore, content: &str, tags: &[&str]) -> Memory {
        database
            .add(&MemoryInput {
                content: content.to_owned(),
                tags: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
                source: Some("test".to_owned()),
            })
            .unwrap()
            .0
    }

    #[test]
    fn a_save_round_trips() {
        let (_directory, database) = database();
        let memory = save(
            &database,
            "USER_NAME is allergic to nuts",
            &["health", "user"],
        );
        assert!(memory.id.starts_with("m_"));
        assert_eq!(memory.revision, 1);
        assert_eq!(memory.tags, vec!["health", "user"]);
        assert_eq!(database.get(&memory.id).unwrap(), memory);
    }

    #[test]
    fn credential_like_content_is_refused() {
        let (_directory, database) = database();
        let error = database
            .add(&MemoryInput {
                content: "token=abcdefghijk".to_owned(),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("credential"), "{}", error.message);
    }

    #[test]
    fn identical_content_is_reused_and_left_untouched() {
        let (_directory, database) = database();
        let first = save(&database, "same fact", &["a"]);
        let (second, created) = database
            .add(&MemoryInput {
                content: "same fact".to_owned(),
                tags: Some(vec!["b".to_owned()]),
                source: Some("elsewhere".to_owned()),
            })
            .unwrap();
        assert!(!created);
        assert_eq!(second.id, first.id);
        // The original keeps the existing tags, source, and revision.
        assert_eq!(second.tags, vec!["a"]);
        assert_eq!(second.revision, 1);
        assert_eq!(database.count().unwrap(), 1);
    }

    #[test]
    fn a_keyword_search_finds_the_memory() {
        let (_directory, database) = database();
        save(&database, "USER_NAME prefers brief replies", &["style"]);
        save(&database, "USER_NAME likes sour soup", &["food"]);
        let response = database
            .search(
                "soup",
                SearchMode::Any,
                20,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].memory.content,
            "USER_NAME likes sour soup"
        );
        assert_eq!(response.mode, "any");
    }

    #[test]
    fn stemming_matches_an_inflected_form() {
        let (_directory, database) = database();
        save(&database, "she was painting a sunrise", &[]);
        let response = database
            .search(
                "paint",
                SearchMode::Any,
                20,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(response.results.len(), 1);
    }

    #[test]
    fn an_all_search_requires_every_term() {
        let (_directory, database) = database();
        save(&database, "sour pork soup", &[]);
        save(&database, "sweet pork stew", &[]);
        let response = database
            .search(
                "sour soup",
                SearchMode::All,
                20,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(response.results.len(), 1);

        // Neither memory holds both terms, so the answer is nothing. When the
        // index finds no row the search falls back to a literal scan of the whole
        // query string, which is why the reported mode changes.
        let none = database
            .search(
                "sour stew",
                SearchMode::All,
                20,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert!(none.results.is_empty());
        assert_eq!(none.mode, "literal");
    }

    #[test]
    fn a_substring_the_tokenizer_cannot_match_falls_back_to_literal() {
        let (_directory, database) = database();
        save(&database, "the code is PC2024-0037 annex", &[]);
        // "0037" is its own token, so the index answers this one directly.
        let indexed = database
            .search(
                "0037",
                SearchMode::Any,
                20,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(indexed.results.len(), 1);
        assert_eq!(indexed.mode, "any");

        // "C2024" begins mid-token, so no prefix match exists. The literal
        // fallback is what keeps a plainly-present substring from being reported
        // as absent.
        let response = database
            .search(
                "C2024",
                SearchMode::Any,
                20,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.mode, "literal");
    }

    #[test]
    fn tag_filters_apply_before_pagination() {
        let (_directory, database) = database();
        save(&database, "one apple", &["fruit"]);
        save(&database, "two apples", &["fruit"]);
        save(&database, "three carrots", &["vegetable"]);
        let response = database
            .search(
                "apple",
                SearchMode::Any,
                1,
                0,
                &MemorySearchOptions {
                    tags: vec!["fruit".to_owned()],
                },
            )
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert!(response.next_offset.is_some());
        assert_eq!(response.tags, vec!["fruit"]);
    }

    #[test]
    fn an_empty_query_browses_every_memory() {
        let (_directory, database) = database();
        save(&database, "first", &[]);
        save(&database, "second", &[]);
        let response = database
            .search("", SearchMode::Any, 20, 0, &MemorySearchOptions::default())
            .unwrap();
        assert_eq!(response.mode, "browse");
        assert_eq!(response.results.len(), 2);
    }

    #[test]
    fn pagination_reports_next_offset_and_then_stops() {
        let (_directory, database) = database();
        for index in 0..5 {
            save(&database, &format!("memory number {index}"), &[]);
        }
        let first = database
            .search(
                "memory",
                SearchMode::Any,
                2,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(first.results.len(), 2);
        assert_eq!(first.next_offset, Some(2));

        let last = database
            .search(
                "memory",
                SearchMode::Any,
                2,
                4,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(last.results.len(), 1);
        assert_eq!(last.next_offset, None);
    }

    #[test]
    fn a_stale_revision_is_rejected() {
        let (_directory, database) = database();
        let memory = save(&database, "original", &[]);
        let error = database
            .edit(&memory.id, Some("changed"), None, None, Some(99))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(database.get(&memory.id).unwrap().content, "original");
    }

    #[test]
    fn an_edit_bumps_the_revision_and_preserves_omitted_fields() {
        let (_directory, database) = database();
        let memory = save(&database, "original", &["keep"]);
        let edited = database
            .edit(&memory.id, Some("changed"), None, None, Some(1))
            .unwrap();
        assert_eq!(edited.content, "changed");
        assert_eq!(edited.tags, vec!["keep"]);
        assert_eq!(edited.source, "test");
        assert_eq!(edited.revision, 2);
    }

    #[test]
    fn a_delete_returns_what_was_removed_and_then_404s() {
        let (_directory, database) = database();
        let memory = save(&database, "temporary", &[]);
        let deleted = database.delete(&memory.id, None).unwrap();
        assert_eq!(deleted.id, memory.id);
        assert_eq!(
            database.get(&memory.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn an_archived_bullet_is_stored_with_its_provenance() {
        let (_directory, database) = database();
        let entry = StandingItem {
            id: "s_abc".to_owned(),
            priority: 10,
            content: "evicted bullet".to_owned(),
        };
        database.archive(&[entry], "/home/standing.md").unwrap();
        let response = database
            .search(
                "evicted",
                SearchMode::Any,
                20,
                0,
                &MemorySearchOptions::default(),
            )
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].memory.tags, vec!["standing-archive"]);
        assert!(
            response.results[0]
                .memory
                .source
                .starts_with("/home/standing.md#s_abc")
        );
    }

    #[test]
    fn archiving_twice_does_not_duplicate() {
        let (_directory, database) = database();
        let entry = StandingItem {
            id: "s_abc".to_owned(),
            priority: 10,
            content: "evicted bullet".to_owned(),
        };
        database
            .archive(std::slice::from_ref(&entry), "/home/standing.md")
            .unwrap();
        database.archive(&[entry], "/home/standing.md").unwrap();
        assert_eq!(database.count().unwrap(), 1);
    }

    #[test]
    fn an_oversized_archived_bullet_is_split_into_parts() {
        let (_directory, database) = database();
        let entry = StandingItem {
            id: "s_big".to_owned(),
            priority: 10,
            content: "x".repeat(40_000),
        };
        database.archive(&[entry], "/home/standing.md").unwrap();
        assert_eq!(database.count().unwrap(), 3);
    }

    #[test]
    fn a_reopened_database_keeps_its_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("long-term.sqlite");
        {
            let database = MemoryStore::open(&path).unwrap();
            save(&database, "durable", &[]);
        }
        let reopened = MemoryStore::open(&path).unwrap();
        assert_eq!(reopened.count().unwrap(), 1);
    }

    #[test]
    fn a_foreign_database_is_refused_rather_than_adopted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unrelated.sqlite");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE unrelated (id INTEGER PRIMARY KEY); PRAGMA user_version=1;",
                )
                .unwrap();
        }
        let error = MemoryStore::open(&path).unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreUnavailable);
    }

    #[test]
    fn a_term_is_split_on_punctuation() {
        assert_eq!(split_terms("hello, world!"), vec!["hello", "world"]);
        assert_eq!(split_terms("snake_case"), vec!["snake_case"]);
    }
}
