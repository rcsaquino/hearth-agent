//! Consistent database snapshots and their verification.

use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::{StorageError, integrity_check, open_read_only, require_schema_version};

/// The folder a snapshot is written in before it is renamed into place.
///
/// SQLite creates the vacuum target with the process umask, so the file is
/// world-readable for the life of the copy. A private folder makes the file
/// unreachable to another local user for that window, and the file is chmodded
/// to 0600 before the rename.
fn staging_folder(destination: &Path) -> std::path::PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot.db");
    destination.with_file_name(format!(".{file_name}.snapshot"))
}

#[cfg(unix)]
fn create_private_folder(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_folder(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

/// Copy the database into a consistent snapshot, replacing the destination.
///
/// The snapshot is written inside a private staging folder, secured at mode
/// 0600, then renamed into place. A failure leaves no partial destination and no
/// staging folder.
pub fn snapshot(database: &Path, destination: &Path) -> Result<(), StorageError> {
    if !database.is_file() {
        return Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("database not found: {}", database.display()),
        )));
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = staging_folder(destination);
    let _ = fs::remove_dir_all(&staging);
    if let Err(error) = create_private_folder(&staging) {
        return Err(error.into());
    }

    let connection = Connection::open(database)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let temporary = staging.join("snapshot.db");
    let temporary_text = temporary.to_string_lossy().into_owned();
    if let Err(error) = connection.execute("VACUUM INTO ?", [temporary_text.as_str()]) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    drop(connection);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    let _ = fs::remove_dir_all(&staging);
    Ok(())
}

/// Verify a snapshot's integrity and schema version.
pub fn verify(path: &Path) -> Result<i64, StorageError> {
    let connection = open_read_only(path)?;
    integrity_check(&connection)?;
    require_schema_version(&connection)
}
