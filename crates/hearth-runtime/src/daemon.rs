use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use hearth_config::ConfigError;
use hearth_core::{CoreError, Harness};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use thiserror::Error;

use crate::effect_worker::EffectWorker;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("another hearth daemon is already running (pid {0})")]
    AlreadyRunning(u32),
    #[error("another hearth daemon is already running")]
    AlreadyRunningUnknown,
    #[error("daemon file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Acquire the single-writer lock, recover interrupted work, and wait.
pub fn daemon(home: &Path) -> Result<(), DaemonError> {
    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;

    let lock_path = home.join("hearth.lock");
    let mut lock = open_lock(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(match read_pid(&lock_path) {
                Some(pid) => DaemonError::AlreadyRunning(pid),
                None => DaemonError::AlreadyRunningUnknown,
            });
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    lock.set_len(0)?;
    writeln!(lock, "{}", std::process::id())?;
    lock.flush()?;

    let harness = Harness::open(home)?;
    let session = harness.session().clone();
    let recovery = session.recover_tasks()?;
    tracing::info!(
        requeued = recovery.requeued,
        poisoned = recovery.poisoned,
        woken = recovery.woken,
        "task recovery complete"
    );

    let workspace = match settings.agent.workspace.clone() {
        Some(path) if path.is_absolute() => path,
        Some(path) => home.join(path),
        None => home.join("workspace"),
    };
    let worker = EffectWorker::new(session, workspace);
    let recovered = worker.recover_started()?;
    tracing::info!(recovered, "effect recovery complete");

    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);
    let handle = thread::spawn(move || {
        while !worker_shutdown.load(Ordering::Relaxed) {
            if let Err(error) = worker.run_once() {
                tracing::error!(%error, "effect worker iteration failed");
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    tracing::info!("effect worker started");

    tracing::info!(
        pid = std::process::id(),
        home = %home.display(),
        "hearth daemon started"
    );
    let mut signals = Signals::new([SIGTERM, SIGINT])?;
    if let Some(signal) = signals.forever().next() {
        tracing::info!(signal, "hearth daemon stopping");
    }
    shutdown.store(true, Ordering::Relaxed);
    if handle.join().is_err() {
        tracing::error!("effect worker panicked");
    }
    drop(harness);
    Ok(())
}

fn open_lock(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
