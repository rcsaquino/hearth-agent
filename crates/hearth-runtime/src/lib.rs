//! Hearth runtime wiring.

use std::path::Path;

mod agent;
mod chat;
mod compaction;
mod daemon;
mod doctor;
mod effect_worker;
mod helpers;
mod events;
mod init;
mod ipc;
mod memory_extract;
mod prompt_audit;
mod scheduler;
mod stats;
pub mod tool_groups;
mod tools;

pub use agent::{Agent, AgentConfig, AgentError, RunOutcome};
pub use chat::{ChatError, chat};
pub use compaction::{CompactionError, Compactor};
pub use daemon::{DaemonError, daemon};
pub use doctor::{DoctorError, doctor};
pub use effect_worker::EffectWorker;
pub use helpers::{HelperConfig, HelperControls, HelperPool, TaskTool};
pub use events::{EventBus, EventSink, NoopSink, RunEvent};
pub use init::{InitError, InitOptions, init};
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
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::sync::Mutex::new(file))
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}
