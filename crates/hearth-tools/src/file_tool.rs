//! The files tool: read, write, edit, and list inside a workspace.

use hearth_model::ToolSpec;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// File access confined to a workspace, with spill reads.
#[derive(Debug, Clone)]
pub struct FileTool {
    workspace: Workspace,
}

impl FileTool {
    /// Build a file tool.
    pub fn new(
        workspace: impl Into<std::path::PathBuf>,
        spill: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            workspace: Workspace::new(workspace, spill),
        }
    }
}

impl Tool for FileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "files".to_owned(),
            description: "Read, write, edit, or list files in the workspace. Use the spill/ prefix to read oversized tool output."
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["read", "write", "edit", "list"]},
                    "path": {"type": "string", "description": "Relative path inside the workspace"},
                    "content": {"type": "string"},
                    "old": {"type": "string"},
                    "new": {"type": "string"},
                    "offset": {"type": "integer", "minimum": 1},
                    "limit": {"type": "integer", "minimum": 1}
                },
                "required": ["action"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "action is required"))?;
        match action {
            "read" => {
                let path = required_path(arguments)?;
                let text = self.workspace.read_text(path)?;
                let content = slice_lines(&text, arguments);
                Ok(ToolOutput {
                    content,
                    is_error: false,
                })
            }
            "write" => {
                let path = required_path(arguments)?;
                let content = arguments
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "content is required for write")
                    })?;
                self.workspace.write_text(path, content)?;
                Ok(ToolOutput {
                    content: format!("wrote {} bytes to {path}", content.len()),
                    is_error: false,
                })
            }
            "edit" => {
                let path = required_path(arguments)?;
                let old = arguments.get("old").and_then(Value::as_str).ok_or_else(|| {
                    ToolError::invalid("invalid_arguments", "old is required for edit")
                })?;
                let new = arguments.get("new").and_then(Value::as_str).ok_or_else(|| {
                    ToolError::invalid("invalid_arguments", "new is required for edit")
                })?;
                let text = self.workspace.read_text(path)?;
                if !text.contains(old) {
                    return Err(ToolError::invalid(
                        "not_found",
                        "the old text was not found",
                    ));
                }
                let updated = text.replacen(old, new, 1);
                self.workspace.write_text(path, &updated)?;
                Ok(ToolOutput {
                    content: format!("edited {path}"),
                    is_error: false,
                })
            }
            "list" => {
                let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
                let entries = self.workspace.read_dir(path)?;
                let mut content = String::new();
                for entry in entries {
                    content.push_str(&format!(
                        "{}\t{}\t{}\n",
                        entry.kind, entry.name, entry.size
                    ));
                }
                Ok(ToolOutput {
                    content,
                    is_error: false,
                })
            }
            other => Err(ToolError::invalid(
                "unknown_action",
                format!("unknown files action: {other}"),
            )),
        }
    }
}

fn required_path(arguments: &Value) -> Result<&str, ToolError> {
    arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::invalid("invalid_arguments", "path is required"))
}

fn slice_lines(text: &str, arguments: &Value) -> String {
    let offset = arguments
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = arguments.get("limit").and_then(Value::as_u64);
    if offset == 1 && limit.is_none() {
        return text.to_owned();
    }
    let lines: Vec<&str> = text.lines().collect();
    let start = offset.saturating_sub(1).min(lines.len());
    let end = match limit {
        Some(limit) => start.saturating_add(limit as usize).min(lines.len()),
        None => lines.len(),
    };
    let mut selected = lines[start..end].join("\n");
    if !selected.is_empty() {
        selected.push('\n');
    }
    selected
}
