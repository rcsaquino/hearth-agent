//! Hearth storage: SQLite connections and migrations.

mod schema;
mod storage;

use std::path::Path;

pub use rusqlite;
use rusqlite::{Connection, OpenFlags};
use thiserror::Error;

/// The schema version this build understands.
pub const SCHEMA_VERSION: i64 = 1;

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

/// Apply every migration the database is missing.
pub fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        0 => {
            let transaction = connection.transaction()?;
            transaction.execute_batch(schema::MIGRATION_0001)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        SCHEMA_VERSION => {}
        found => {
            return Err(StorageError::SchemaVersion {
                found,
                expected: SCHEMA_VERSION,
            });
        }
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
