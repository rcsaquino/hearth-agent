//! Consistent database snapshots and their verification.

use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::{StorageError, integrity_check, open_read_only, require_schema_version};

/// Copy the database into a consistent snapshot, replacing the destination.
///
/// The snapshot is written to a temporary file in the destination folder,
/// secured at mode 0600, then renamed into place. A failure leaves no partial
/// destination.
pub fn snapshot(database: &Path, destination: &Path) -> Result<(), StorageError> {
    if !database.is_file() {
        return Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("database not found: {}", database.display()),
        )));
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot.db");
    let temporary = destination.with_file_name(format!(".{file_name}.tmp"));
    let _ = fs::remove_file(&temporary);

    let connection = Connection::open(database)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let temporary_text = temporary.to_string_lossy().into_owned();
    if let Err(error) = connection.execute("VACUUM INTO ?", [temporary_text.as_str()]) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    drop(connection);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

/// Verify a snapshot's integrity and schema version.
pub fn verify(path: &Path) -> Result<i64, StorageError> {
    let connection = open_read_only(path)?;
    integrity_check(&connection)?;
    require_schema_version(&connection)
}
