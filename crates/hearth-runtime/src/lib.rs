//! Hearth runtime wiring.

use std::io::Write as _;
use std::path::Path;

mod agent;
pub mod backup;
mod compaction;
mod daemon;
mod doctor;
mod events;
pub mod extraction;
mod helpers;
mod home;
mod init;
pub mod inspect;
mod ipc;
pub mod maintenance;
mod scheduler;
pub mod service;
pub mod skill_review;
pub mod skills;
pub mod spill;
pub mod tool_groups;
mod tools;

pub use agent::{Agent, AgentConfig, AgentError, RunOutcome};
pub use compaction::{CompactionError, Compactor};
pub use daemon::{DaemonError, daemon};
pub use doctor::{DoctorError, doctor_with};
pub use events::{EventBus, EventSink, NoopSink, RunEvent};
pub use hearth_core::{TokenCounts, Zone, cache_hit_rate};
pub use helpers::{HelperConfig, HelperControls, HelperPool, TaskTool};
pub use home::{HomeError, HomeRestore, restore_home};
pub use init::{InitError, InitOptions, InitReport, init};
pub use ipc::{IpcError, StatusContext, open_watch, read_status, serve, socket_path};
pub use scheduler::{
    ReminderHandler, ScheduleTool, Scheduler, TaskHandler, TaskHandlerError, TaskRegistry,
};
pub use tool_groups::{ToolGroup, ToolGroups, ToolSearchTool};
pub use tools::ToolMediaRoots;

/// The size at which `logs/hearth.log` is rotated to `hearth.log.1`.
pub const LOG_LIMIT_BYTES: u64 = 10 * 1024 * 1024;

/// Install the structured JSON logger used by the daemon.
pub fn init_logging(home: &Path) -> std::io::Result<()> {
    let logs = home.join("logs");
    std::fs::create_dir_all(&logs)?;
    let path = logs.join("hearth.log");
    let file = std::sync::Arc::new(std::sync::Mutex::new(LogFile::open(
        &path,
        LOG_LIMIT_BYTES,
    )?));
    let settings = hearth_config::load(home).ok();
    let secrets = std::sync::Arc::new(
        settings
            .as_ref()
            .map(daemon::collect_secrets)
            .unwrap_or_default(),
    );
    let level = settings
        .as_ref()
        .and_then(|settings| log_level(&settings.logging.level))
        .unwrap_or(tracing::Level::INFO);
    let writer_file = std::sync::Arc::clone(&file);
    let writer_secrets = std::sync::Arc::clone(&secrets);
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(level)
        .with_writer(move || RedactingWriter {
            file: std::sync::Arc::clone(&writer_file),
            secrets: std::sync::Arc::clone(&writer_secrets),
        })
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}

fn log_level(name: &str) -> Option<tracing::Level> {
    Some(match name {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => return None,
    })
}

/// One append-only log file that rotates into a single `.1` copy when full.
struct LogFile {
    file: std::fs::File,
    path: std::path::PathBuf,
    size: u64,
    limit: u64,
}

impl LogFile {
    fn open(path: &Path, limit: u64) -> std::io::Result<Self> {
        let file = open_log(path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            file,
            path: path.to_path_buf(),
            size,
            limit,
        })
    }

    fn append(&mut self, buffer: &[u8]) -> std::io::Result<()> {
        if self.size > 0 && self.size + buffer.len() as u64 > self.limit {
            self.rotate()?;
        }
        self.file.write_all(buffer)?;
        self.size += buffer.len() as u64;
        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        let rotated = self.path.with_extension("log.1");
        let _ = std::fs::remove_file(&rotated);
        std::fs::rename(&self.path, &rotated)?;
        self.file = open_log(&self.path)?;
        self.size = 0;
        Ok(())
    }
}

fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// A log writer that redacts credential-shaped tokens before they reach disk.
struct RedactingWriter {
    file: std::sync::Arc<std::sync::Mutex<LogFile>>,
    secrets: std::sync::Arc<Vec<String>>,
}

impl std::io::Write for RedactingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buffer);
        let redacted = hearth_core::secrets::redact_with_values(&text, &self.secrets);
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("log lock is poisoned"))?;
        file.append(redacted.as_bytes())?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("log lock is poisoned"))?;
        file.file.flush()
    }
}

#[cfg(test)]
mod log_rotation_tests {
    use super::*;

    #[test]
    fn a_full_log_rotates_into_one_previous_copy() {
        let home = tempfile::TempDir::new().expect("temp home");
        let logs = home.path().join("logs");
        std::fs::create_dir_all(&logs).expect("logs");
        let path = logs.join("hearth.log");
        std::fs::write(&path, vec![b'x'; 1024]).expect("seed log");

        let mut log = LogFile::open(&path, 512).expect("open log");
        log.append(b"one more line\n").expect("append");

        let rotated = logs.join("hearth.log.1");
        assert_eq!(
            std::fs::metadata(&rotated).expect("rotated file").len(),
            1024
        );
        assert_eq!(
            std::fs::read(&path).expect("current log"),
            b"one more line\n"
        );
    }
}
