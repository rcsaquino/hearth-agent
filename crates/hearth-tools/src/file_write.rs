//! Confined text file writes used by the effect runner.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use thiserror::Error;

/// A tool failure that says whether a retry could succeed.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("{0}")]
    Invalid(String),
    #[error("file write failed: {0}")]
    Io(#[from] std::io::Error),
}

impl ToolError {
    /// Whether the same call could succeed later.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Io(_))
    }
}

/// Writes text files under one workspace root.
#[derive(Clone, Debug)]
pub struct FileWrite {
    root: PathBuf,
}

impl FileWrite {
    /// Create a writer confined to `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Write the text in a `{ path, content }` payload.
    pub fn write(&self, payload: &Value) -> Result<Value, ToolError> {
        let object = payload
            .as_object()
            .ok_or_else(|| ToolError::Invalid("payload must be an object".to_owned()))?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Invalid("payload.path must be a string".to_owned()))?;
        let content = object
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Invalid("payload.content must be a string".to_owned()))?;
        self.write_text(path, content)
    }

    fn write_text(&self, path: &str, content: &str) -> Result<Value, ToolError> {
        if path.trim().is_empty() {
            return Err(ToolError::Invalid("path is empty".to_owned()));
        }
        let relative = Path::new(path);
        if relative.is_absolute() {
            return Err(ToolError::Invalid("path must be relative".to_owned()));
        }
        for component in relative.components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                _ => {
                    return Err(ToolError::Invalid(
                        "path must not contain .. or a root".to_owned(),
                    ));
                }
            }
        }
        if relative.file_name().is_none() {
            return Err(ToolError::Invalid("path has no file name".to_owned()));
        }

        fs::create_dir_all(&self.root)?;
        let root = fs::canonicalize(&self.root)?;
        let target = root.join(relative);
        let parent = target
            .parent()
            .ok_or_else(|| ToolError::Invalid("path has no parent".to_owned()))?;
        fs::create_dir_all(parent)?;
        let canonical_parent = fs::canonicalize(parent)?;
        if !canonical_parent.starts_with(&root) {
            return Err(ToolError::Invalid("path escapes the workspace".to_owned()));
        }
        match fs::symlink_metadata(&target) {
            Ok(_) => {
                let canonical_target = fs::canonicalize(&target)
                    .map_err(|_| ToolError::Invalid("path escapes the workspace".to_owned()))?;
                if !canonical_target.starts_with(&root) {
                    return Err(ToolError::Invalid("path escapes the workspace".to_owned()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&target)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
        }

        Ok(json!({"path": path, "bytes": content.len()}))
    }
}
