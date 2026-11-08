//! Hearth runtime wiring.

use std::path::Path;

mod agent;
mod daemon;
mod doctor;
mod effect_worker;
mod init;

pub use agent::{Agent, AgentConfig, AgentError, RunOutcome};
pub use daemon::{DaemonError, daemon};
pub use doctor::{DoctorError, doctor};
pub use effect_worker::EffectWorker;
pub use init::{InitError, InitOptions, init};

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
