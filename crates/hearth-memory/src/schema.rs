//! Schema for the long-term store.
//!
//! One table, an FTS5 index over it, and triggers that keep the index in step.
//! The table is `STRICT` so a mistyped column cannot silently store the wrong
//! type, and the index uses Porter stemming with a two- and three-character
//! prefix index, which is what lets `paint` match `painting` and lets a prefix
//! search stay fast.
//!
//! The database is signed with [`APPLICATION_ID`] so a file that belongs to
//! something else is never adopted and migrated in place. Everything about this
//! store is ours; there is no compatibility path to another agent's format.

use rusqlite::Connection;

/// Marker written to `PRAGMA application_id`. The bytes spell `HARTH`.
pub const APPLICATION_ID: i64 = 0x4841_5254;

/// The current schema version.
pub const SCHEMA_VERSION: i64 = 1;

/// The full schema.
///
/// `tag_text` is a newline-joined mirror of the JSON `tags` array. It exists
/// because FTS5 indexes columns, not array elements, so the flattened form is
/// what makes a tag searchable while the JSON form stays authoritative.
pub const INITIAL_SCHEMA: &str = r#"
  CREATE TABLE memories (
    rowid INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    tag_text TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1
  ) STRICT;

  CREATE INDEX memories_content ON memories(content);

  CREATE VIRTUAL TABLE memories_fts USING fts5(
    content, tag_text, source, content='memories', content_rowid='rowid',
    tokenize='porter unicode61 remove_diacritics 2', prefix='2 3'
  );

  CREATE TRIGGER memories_insert AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, content, tag_text, source)
      VALUES(new.rowid, new.content, new.tag_text, new.source);
  END;

  CREATE TRIGGER memories_delete AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, tag_text, source)
      VALUES('delete', old.rowid, old.content, old.tag_text, old.source);
  END;

  CREATE TRIGGER memories_update AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, tag_text, source)
      VALUES('delete', old.rowid, old.content, old.tag_text, old.source);
    INSERT INTO memories_fts(rowid, content, tag_text, source)
      VALUES(new.rowid, new.content, new.tag_text, new.source);
  END;
"#;

/// Initialize or verify the schema, returning the version found.
///
/// A fresh file is migrated. An existing file must already be ours: a different
/// `application_id` means the path points at someone else's database, and
/// migrating it would corrupt it. Refusing is the only safe answer.
pub fn prepare(connection: &Connection) -> Result<i64, String> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("cannot read schema version: {error}"))?;
    let application: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|error| format!("cannot read application id: {error}"))?;

    if version > SCHEMA_VERSION {
        return Err(format!(
            "Memory schema {version} is newer than this build supports ({SCHEMA_VERSION})."
        ));
    }
    if version == 0 && application == 0 {
        connection
            .execute_batch(INITIAL_SCHEMA)
            .map_err(|error| format!("cannot create memory schema: {error}"))?;
        connection
            .execute_batch(&format!(
                "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={SCHEMA_VERSION};"
            ))
            .map_err(|error| format!("cannot sign memory database: {error}"))?;
        return Ok(SCHEMA_VERSION);
    }
    if application != APPLICATION_ID {
        return Err(format!(
            "Refusing to use {}: it is not a Hearth memory database.",
            "this file"
        ));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_file_is_created_and_signed() {
        let connection = Connection::open_in_memory().unwrap();
        assert_eq!(prepare(&connection).unwrap(), SCHEMA_VERSION);
        let application: i64 = connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap();
        assert_eq!(application, APPLICATION_ID);
    }

    #[test]
    fn preparing_twice_is_idempotent() {
        let connection = Connection::open_in_memory().unwrap();
        prepare(&connection).unwrap();
        prepare(&connection).unwrap();
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memories'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1);
    }

    #[test]
    fn a_foreign_database_is_refused() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE unrelated (id INTEGER PRIMARY KEY); PRAGMA user_version=1;",
            )
            .unwrap();
        assert!(prepare(&connection).is_err());
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let connection = Connection::open_in_memory().unwrap();
        prepare(&connection).unwrap();
        connection
            .execute_batch(&format!("PRAGMA user_version={};", SCHEMA_VERSION + 1))
            .unwrap();
        let error = prepare(&connection).unwrap_err();
        assert!(error.contains("newer"), "{error}");
    }

    #[test]
    fn the_index_stems_and_matches_a_prefix() {
        let connection = Connection::open_in_memory().unwrap();
        prepare(&connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO memories(id,content,created_at,updated_at) \
                 VALUES('m_1','she was painting a sunrise','now','now');",
            )
            .unwrap();
        let hits: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH '\"paint\"*'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);
    }

    #[test]
    fn a_delete_trigger_keeps_the_index_consistent() {
        let connection = Connection::open_in_memory().unwrap();
        prepare(&connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO memories(id,content,created_at,updated_at) \
                 VALUES('m_1','ephemeral fact','now','now');
                 DELETE FROM memories WHERE id='m_1';",
            )
            .unwrap();
        let hits: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'ephemeral'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
    }
}
