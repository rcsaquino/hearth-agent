//! The shell tool: commands in the workspace with a bounded wait.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use hearth_model::ToolSpec;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

const CAPTURE_CAP: usize = 1024 * 1024;

/// Runs shell commands from the workspace. This is not a sandbox.
#[derive(Debug, Clone)]
pub struct ShellTool {
    workspace: Workspace,
    timeout: Duration,
}

impl ShellTool {
    /// Build a shell tool with a timeout.
    pub fn new(workspace: impl Into<std::path::PathBuf>, timeout: Duration) -> Self {
        Self {
            workspace: Workspace::root_only(workspace),
            timeout,
        }
    }
}

impl Tool for ShellTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell".to_owned(),
            description: "Run a shell command from the workspace. No approval and no sandbox."
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "cwd": {"type": "string", "description": "Optional directory relative to the workspace"}
                },
                "required": ["command"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let command = arguments
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "command is required"))?;
        if command.trim().is_empty() {
            return Err(ToolError::invalid("invalid_arguments", "command is empty"));
        }
        let cwd = match arguments.get("cwd").and_then(Value::as_str) {
            Some(path) => self.workspace.resolve_workspace_directory(path)?,
            None => self.workspace.resolve_workspace_directory(".")?,
        };

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ToolError::from)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::invalid("spawn", "stdout was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::invalid("spawn", "stderr was not captured"))?;
        let stdout_reader = thread::spawn(move || read_capped(stdout));
        let stderr_reader = thread::spawn(move || read_capped(stderr));

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(ToolError::invalid(
                        "timeout",
                        format!("command timed out after {:?}", self.timeout),
                    ));
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(ToolError::from(error));
                }
            }
        };

        let (stdout, stdout_truncated, _) = stdout_reader
            .join()
            .map_err(|_| ToolError::invalid("io", "stdout reader panicked"))?;
        let (stderr, stderr_truncated, _) = stderr_reader
            .join()
            .map_err(|_| ToolError::invalid("io", "stderr reader panicked"))?;

        let mut content = format!(
            "exit code: {}\n--- stdout ---\n{}",
            status.code().unwrap_or(-1),
            stdout
        );
        if stdout_truncated {
            content.push_str(&format!("\n[stdout truncated at {CAPTURE_CAP} bytes]"));
        }
        content.push_str("\n--- stderr ---\n");
        content.push_str(&stderr);
        if stderr_truncated {
            content.push_str(&format!("\n[stderr truncated at {CAPTURE_CAP} bytes]"));
        }
        Ok(ToolOutput {
            content,
            is_error: !status.success(),
        })
    }
}

fn read_capped(mut reader: impl Read) -> (String, bool, u64) {
    let mut buffer = [0u8; 8192];
    let mut stored = Vec::new();
    let mut total = 0u64;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                total += read as u64;
                if stored.len() < CAPTURE_CAP {
                    let take = (CAPTURE_CAP - stored.len()).min(read);
                    stored.extend_from_slice(&buffer[..take]);
                }
            }
            Err(_) => break,
        }
    }
    (
        String::from_utf8_lossy(&stored).to_string(),
        total > CAPTURE_CAP as u64,
        total,
    )
}
