//! Hearth runtime wiring.

mod daemon;
mod doctor;
mod init;

pub use daemon::{DaemonError, daemon};
pub use doctor::{DoctorError, doctor};
pub use init::{InitError, InitOptions, init};

/// Install the structured JSON logger used by the daemon.
pub fn init_logging() {
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}
