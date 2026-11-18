//! Hearth storage: SQLite connections and migrations.

mod backup;
mod schema;
mod storage;

use std::path::Path;

pub use rusqlite;
use rusqlite::{Connection, OpenFlags};
use thiserror::Error;

/// The schema version this build understands.
pub const SCHEMA_VERSION: i64 = 13;

pub use backup::{snapshot, verify};
pub use storage::{Reader, Storage, Writer};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database file could not be secured: {0}")]
    Io(#[from] std::io::Error),
    #[error("database schema version {found} is not supported by this build (expected {expected})")]
    SchemaVersion { found: i64, expected: i64 },
    #[error("database integrity check failed: {0}")]
    Integrity(String),
    #[error("storage writer is closed")]
    Closed,
    #[error("a storage write panicked and was rolled back")]
    Panicked,
    #[error("storage lock is poisoned")]
    Poisoned,
}

impl StorageError {
    /// Whether this error is a unique or primary-key constraint violation.
    pub fn is_unique_violation(&self) -> bool {
        match self {
            StorageError::Sqlite(rusqlite::Error::SqliteFailure(error, _)) => {
                error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    || error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
            }
            _ => false,
        }
    }
}

/// Abort at a named crash point when `HEARTH_CRASH_AT` matches.
///
/// This is the test hook used by the S7 crash suite. It is inert unless the
/// environment variable names the point.
pub fn crash_if(point: &str) {
    if std::env::var("HEARTH_CRASH_AT").ok().as_deref() == Some(point) {
        std::process::abort();
    }
}

/// Open a database for writing, apply connection settings, and migrate it.
pub fn open(path: &Path) -> Result<Connection, StorageError> {
    let mut connection = Connection::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    configure(&connection)?;
    migrate(&mut connection)?;
    Ok(connection)
}

/// Open a database without creating or changing it.
pub fn open_read_only(path: &Path) -> Result<Connection, StorageError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<(), StorageError> {
    connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "busy_timeout", 5000)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    Ok(())
}

/// Every migration, as the version it upgrades and the version it produces.
const MIGRATIONS: [(i64, &str, i64); 13] = [
    (0, schema::MIGRATION_0001, 1),
    (1, schema::MIGRATION_0002, 2),
    (2, schema::MIGRATION_0003, 3),
    (3, schema::MIGRATION_0004, 4),
    (4, schema::MIGRATION_0005, 5),
    (5, schema::MIGRATION_0006, 6),
    (6, schema::MIGRATION_0007, 7),
    (7, schema::MIGRATION_0008, 8),
    (8, schema::MIGRATION_0009, 9),
    (9, schema::MIGRATION_0010, 10),
    (10, schema::MIGRATION_0011, 11),
    (11, schema::MIGRATION_0012, 12),
    (12, schema::MIGRATION_0013, 13),
];

/// Apply every migration the database is missing.
pub fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StorageError::SchemaVersion {
            found: version,
            expected: SCHEMA_VERSION,
        });
    }
    let mut current = version;
    for (from, sql, to) in MIGRATIONS {
        if current != from {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", to)?;
        transaction.commit()?;
        current = to;
    }
    if current != SCHEMA_VERSION {
        return Err(StorageError::SchemaVersion {
            found: current,
            expected: SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Read the schema version recorded in the database.
pub fn schema_version(connection: &Connection) -> Result<i64, StorageError> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Verify that the database is at the version this build expects.
pub fn require_schema_version(connection: &Connection) -> Result<i64, StorageError> {
    let found = schema_version(connection)?;
    if found != SCHEMA_VERSION {
        return Err(StorageError::SchemaVersion {
            found,
            expected: SCHEMA_VERSION,
        });
    }
    Ok(found)
}

/// Run SQLite's integrity check.
pub fn integrity_check(connection: &Connection) -> Result<(), StorageError> {
    let result: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(StorageError::Integrity(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_nine_unpins_a_retired_rule() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut connection =
            Connection::open(directory.path().join("hearth.db")).expect("open database");
        // The schema as the build that retired the row left it, before 0009.
        for &(_, sql, _) in MIGRATIONS.iter().filter(|(_, _, to)| *to <= 8) {
            connection.execute_batch(sql).expect("apply migration");
        }
        connection
            .pragma_update(None, "user_version", 8)
            .expect("stamp the older version");
        connection
            .execute_batch(
                "INSERT INTO memories (id, kind, content, fingerprint, pinned, source_kind, \
                 valid_from, valid_to, created_at, updated_at) VALUES \
                 ('m_retired', 'rule', 'never force push', 'f_retired', 1, 'user', 1, 2, 1, 1), \
                 ('m_live', 'rule', 'never deploy late', 'f_live', 1, 'user', 1, NULL, 1, 1);",
            )
            .expect("seed rows");

        migrate(&mut connection).expect("migrate");

        let pinned: i64 = connection
            .query_row(
                "SELECT pinned FROM memories WHERE id = 'm_retired'",
                [],
                |row| row.get(0),
            )
            .expect("retired row");
        assert_eq!(pinned, 0, "a retired rule is not pinned");
        let pinned: i64 = connection
            .query_row(
                "SELECT pinned FROM memories WHERE id = 'm_live'",
                [],
                |row| row.get(0),
            )
            .expect("live row");
        assert_eq!(pinned, 1, "a live rule keeps its pin");
    }
}
