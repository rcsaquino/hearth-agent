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

    println!("settings: ok");
    println!("database: ok (schema {version})");
    Ok(())
}
