//! The files tool: read, write, edit, and list inside a workspace.

use hearth_model::ToolSpec;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// Lines returned by `files read` when the caller gives no limit.
const DEFAULT_READ_LINES: usize = 400;
/// Hard ceiling on one `files read`, whatever the caller asks for.
const MAX_READ_LINES: usize = 2_000;

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
                let old = arguments
                    .get("old")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "old is required for edit")
                    })?;
                let new = arguments
                    .get("new")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
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
                    content.push_str(&format!("{}\t{}\t{}\n", entry.kind, entry.name, entry.size));
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
    let lines: Vec<&str> = text.lines().collect();
    let start = offset.saturating_sub(1).min(lines.len());
    // An unbounded read used to return the whole file, letting one large file
    // flood the transcript. Cap the default and say how to continue.
    let requested = limit.map_or(DEFAULT_READ_LINES, |limit| limit as usize);
    let capped = requested.min(MAX_READ_LINES);
    let end = start.saturating_add(capped).min(lines.len());
    if start == 0 && end == lines.len() {
        // The whole file fits: return it byte for byte, with no added newline.
        return text.to_owned();
    }
    let mut selected = lines[start..end].join("\n");
    if !selected.is_empty() {
        selected.push('\n');
    }
    if end < lines.len() {
        selected.push_str(&format!(
            "[showing lines {}-{} of {}; continue with offset {}]\n",
            start + 1,
            end,
            lines.len(),
            end + 1
        ));
    }
    selected
}
