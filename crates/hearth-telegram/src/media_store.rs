//! Inbound media files, stored under one date directory per UTC day.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;
use time::OffsetDateTime;

/// A media storage failure.
#[derive(Debug, Error)]
pub enum MediaStoreError {
    #[error("media file could not be written: {0}")]
    Io(#[from] std::io::Error),
    #[error("media name is invalid: {0}")]
    Invalid(String),
}

/// The inbound media root, normally `$HEARTH_HOME/media/inbound`.
#[derive(Debug, Clone)]
pub struct InboundMedia {
    root: PathBuf,
}

impl InboundMedia {
    /// Build a store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Save one file under `YYYY-MM-DD/` and return its path.
    pub fn save(
        &self,
        date_unix_seconds: i64,
        message_id: i64,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, MediaStoreError> {
        let date = OffsetDateTime::from_unix_timestamp(date_unix_seconds)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let day = format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            u8::from(date.month()),
            date.day()
        );
        let safe = sanitize(file_name);
        if safe.contains('/') || safe.contains('\\') {
            return Err(MediaStoreError::Invalid(file_name.to_owned()));
        }
        let directory = self.root.join(day);
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{message_id}-{safe}"));
        if path.parent() != Some(directory.as_path()) {
            return Err(MediaStoreError::Invalid(file_name.to_owned()));
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(bytes)?;
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(path)
    }

    /// The root this store writes under.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn sanitize(name: &str) -> String {
    let mut safe: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    if safe.is_empty() || safe == "." || safe == ".." {
        safe = "file".to_owned();
    }
    safe
}
