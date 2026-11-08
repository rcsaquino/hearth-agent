//! Hearth runtime wiring.

use std::path::Path;

mod daemon;
mod doctor;
mod init;

pub use daemon::{DaemonError, daemon};
pub use doctor::{DoctorError, doctor};
pub use init::{InitError, InitOptions, init};

/// Install the structured JSON logger used by the daemon.
pub fn init_logging(home: &Path) -> std::io::Result<()> {
    let logs = home.join("logs");
    std::fs::create_dir_all(&logs)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("hearth.log"))?;
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::sync::Mutex::new(file))
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}
