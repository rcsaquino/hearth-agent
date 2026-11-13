//! The shell tool: commands in the workspace with a bounded wait.

use std::io::{ErrorKind, Read};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use hearth_model::ToolSpec;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

const CAPTURE_CAP: usize = 1024 * 1024;
const DRAIN_GRACE: Duration = Duration::from_millis(250);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

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

        let mut process = Command::new("sh");
        // A descendant can keep a pipe open after the shell exits. Sockets let
        // the parent drain without blocking and stop after a bounded grace.
        let (stdout_stream, stdout_child) = UnixStream::pair().map_err(ToolError::from)?;
        let (stderr_stream, stderr_child) = UnixStream::pair().map_err(ToolError::from)?;
        process
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(stdout_child)))
            .stderr(Stdio::from(OwnedFd::from(stderr_child)));
        #[cfg(unix)]
        process.process_group(0);
        let mut child = process.spawn().map_err(ToolError::from)?;
        let mut stdout = Capture::new(stdout_stream)?;
        let mut stderr = Capture::new(stderr_stream)?;

        let deadline = Instant::now() + self.timeout;
        let mut drain_deadline = None;
        let mut status = None;
        loop {
            let stdout_closed = stdout.drain();
            let stderr_closed = stderr.drain();
            if status.is_none() {
                match child.try_wait() {
                    Ok(Some(value)) => {
                        status = Some(value);
                        drain_deadline = Some(Instant::now() + DRAIN_GRACE);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        terminate(&mut child);
                        return Err(ToolError::from(error));
                    }
                }
            }
            let now = Instant::now();
            match status {
                Some(_) if stdout_closed && stderr_closed => break,
                Some(_) if now >= drain_deadline.unwrap_or(now) => break,
                None if now >= deadline => {
                    terminate(&mut child);
                    let _ = stdout.drain();
                    let _ = stderr.drain();
                    return Err(ToolError::invalid(
                        "timeout",
                        format!("command timed out after {:?}", self.timeout),
                    ));
                }
                _ => thread::sleep(POLL_INTERVAL),
            }
        }

        let status = status.expect("the child status is known when the loop ends");
        let (stdout, stdout_truncated) = stdout.finish();
        let (stderr, stderr_truncated) = stderr.finish();

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

fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // The child leads its own process group. Kill the group so a
        // grandchild cannot keep the capture open past the timeout.
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg("--")
            .arg(format!("-{}", child.id()))
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

struct Capture {
    stream: UnixStream,
    stored: Vec<u8>,
    total: u64,
    closed: bool,
}

impl Capture {
    fn new(stream: UnixStream) -> Result<Self, ToolError> {
        stream.set_nonblocking(true).map_err(ToolError::from)?;
        Ok(Self {
            stream,
            stored: Vec::new(),
            total: 0,
            closed: false,
        })
    }

    /// Drain what is available without waiting for a descendant to close the
    /// capture. Returns true once the stream is closed.
    fn drain(&mut self) -> bool {
        if self.closed {
            return true;
        }
        let mut buffer = [0u8; 8192];
        // Bound the work so the caller still polls the child and the deadline.
        for _ in 0..64 {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(read) => {
                    self.total += read as u64;
                    if self.stored.len() < CAPTURE_CAP {
                        let take = (CAPTURE_CAP - self.stored.len()).min(read);
                        self.stored.extend_from_slice(&buffer[..take]);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
        self.closed
    }

    fn finish(self) -> (String, bool) {
        (
            String::from_utf8_lossy(&self.stored).to_string(),
            self.total > CAPTURE_CAP as u64,
        )
    }
}
