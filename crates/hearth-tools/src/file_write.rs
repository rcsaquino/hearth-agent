//! Confined text file writes used by the effect runner.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::tool::ToolError;
use crate::workspace::Workspace;

/// Writes text files under one workspace root.
#[derive(Clone, Debug)]
pub struct FileWrite {
    workspace: Workspace,
}

impl FileWrite {
    /// Create a writer confined to `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            workspace: Workspace::root_only(root),
        }
    }

    /// Write the text in a `{ path, content }` payload.
    pub fn write(&self, payload: &Value) -> Result<Value, ToolError> {
        let object = payload
            .as_object()
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "payload must be an object"))?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "payload.path must be a string"))?;
        let content = object
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ToolError::invalid("invalid_arguments", "payload.content must be a string")
            })?;
        self.workspace.write_text(path, content)?;
        Ok(json!({"path": path, "bytes": content.len()}))
    }
}
