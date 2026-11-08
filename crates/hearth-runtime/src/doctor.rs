use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DoctorError {
    #[error(transparent)]
    Config(#[from] hearth_config::ConfigError),
    #[error(transparent)]
    Storage(#[from] hearth_storage::StorageError),
    #[error("database not found at {0}; run hearth init")]
    DatabaseMissing(PathBuf),
    #[error("doctor file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Check the home folder without printing secrets.
pub fn doctor(home: &Path) -> Result<(), DoctorError> {
    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;

    let database = home.join("hearth.db");
    if !database.is_file() {
        return Err(DoctorError::DatabaseMissing(database));
    }
    let connection = hearth_storage::open_read_only(&database)?;
    hearth_storage::integrity_check(&connection)?;
    let version = hearth_storage::require_schema_version(&connection)?;

    let lock = lock_state(home)?;

    println!("settings: ok");
    println!("database: ok (schema {version})");
    println!("lock: {lock}");
    Ok(())
}

fn lock_state(home: &Path) -> Result<String, DoctorError> {
    let path = home.join("hearth.lock");
    if !path.exists() {
        return Ok("not held".to_owned());
    }
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok("not held".to_owned()),
        Err(std::fs::TryLockError::WouldBlock) => {
            let pid = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            Ok(match pid {
                Some(pid) => format!("held (pid {pid})"),
                None => "held".to_owned(),
            })
        }
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}
