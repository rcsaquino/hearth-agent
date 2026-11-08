//! Hearth runtime wiring.

use std::path::Path;

mod agent;
pub mod backup;
mod chat;
mod compaction;
mod daemon;
mod doctor;
mod effect_worker;
mod events;
mod helpers;
mod init;
mod ipc;
pub mod maintenance;
mod memory_extract;
mod prompt_audit;
mod scheduler;
pub mod service;
pub mod skill_review;
pub mod skills;
mod stats;
pub mod tool_groups;
mod tools;

pub use agent::{Agent, AgentConfig, AgentError, RunOutcome};
pub use chat::{ChatError, chat};
pub use compaction::{CompactionError, Compactor};
pub use daemon::{DaemonError, daemon};
pub use doctor::{DoctorError, doctor, doctor_with};
pub use effect_worker::EffectWorker;
pub use events::{EventBus, EventSink, NoopSink, RunEvent};
pub use helpers::{HelperConfig, HelperControls, HelperPool, TaskTool};
pub use init::{InitError, InitOptions, init, init_with_io};
pub use ipc::{IpcError, StatusContext, serve, status, watch};
pub use prompt_audit::{PromptError, PromptOptions, prompt};
pub use scheduler::{
    ReminderHandler, ScheduleTool, Scheduler, TaskHandler, TaskHandlerError, TaskRegistry,
};
pub use stats::{StatsError, StatsReport, StatsRow, stats, stats_report};
pub use tool_groups::{ToolGroup, ToolGroups, ToolSearchTool};

/// Install the structured JSON logger used by the daemon.
pub fn init_logging(home: &Path) -> std::io::Result<()> {
    let logs = home.join("logs");
    std::fs::create_dir_all(&logs)?;
    let path = logs.join("hearth.log");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    let file = std::sync::Arc::new(std::sync::Mutex::new(file));
    let writer_file = std::sync::Arc::clone(&file);
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || RedactingWriter {
            file: std::sync::Arc::clone(&writer_file),
        })
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}

/// A log writer that redacts credential-shaped tokens before they reach disk.
struct RedactingWriter {
    file: std::sync::Arc<std::sync::Mutex<std::fs::File>>,
}

impl std::io::Write for RedactingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buffer);
        let redacted = hearth_core::secrets::redact_secrets(&text);
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("log lock is poisoned"))?;
        file.write_all(redacted.as_bytes())?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("log lock is poisoned"))?;
        file.flush()
    }
}
