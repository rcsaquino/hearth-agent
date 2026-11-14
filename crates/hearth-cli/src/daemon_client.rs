//! Daemon probing, lifecycle, and ownership.

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How the current daemon came to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    Attached,
    Spawned(u32),
    ServiceManaged,
}

/// The visible daemon state.
#[derive(Debug, Clone)]
pub enum DaemonState {
    Running,
    Stopped,
    Starting,
    Stopping,
    Error(String),
}

pub fn probe(home: &Path) -> Result<Value, String> {
    hearth_runtime::read_status(home).map_err(|error| error.to_string())
}

pub fn unit_installed(home: &Path) -> bool {
    hearth_runtime::service::status(home)
        .map(|status| status.installed)
        .unwrap_or(false)
}

/// Ensure a daemon is running and say who owns it.
pub fn ensure(home: &Path, start: &str) -> Result<(Ownership, Value), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not find the Hearth binary: {error}"))?;
    ensure_with(home, start, &executable)
}

/// Ensure a daemon is running using a chosen executable.
pub fn ensure_with(
    home: &Path,
    start: &str,
    executable: &Path,
) -> Result<(Ownership, Value), String> {
    if let Ok(status) = probe(home) {
        return Ok((Ownership::Attached, status));
    }
    if let Some(status) = wait_for_existing_daemon(home) {
        return Ok((Ownership::Attached, status));
    }
    let installed = unit_installed(home);
    if installed && start != "tui" {
        // A unit that is still active can hold a daemon whose home was wiped or
        // whose settings changed underneath it. Restarting re-reads both; on an
        // inactive unit systemd treats restart as a start.
        hearth_runtime::service::restart().map_err(|error| error.to_string())?;
        let status = wait_for_status(home)?;
        return Ok((Ownership::ServiceManaged, status));
    }
    let mut child = spawn_detached(executable, home)?;
    match wait_for_status(home) {
        Ok(status) => {
            let pid = lock_pid(home).unwrap_or_else(|| child.id());
            Ok((Ownership::Spawned(pid), status))
        }
        Err(error) => {
            let _ = child.kill();
            Err(error)
        }
    }
}

fn spawn_detached(executable: &Path, home: &Path) -> Result<std::process::Child, String> {
    let mut command = Command::new("setsid");
    command
        .arg(executable)
        .env("HEARTH_DAEMON", "1")
        .env("HEARTH_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(child) => Ok(child),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut fallback = Command::new(executable);
            fallback
                .env("HEARTH_DAEMON", "1")
                .env("HEARTH_HOME", home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| format!("could not start the daemon: {error}"))
        }
        Err(error) => Err(format!("could not start the daemon: {error}")),
    }
}

fn lock_pid(home: &Path) -> Option<u32> {
    std::fs::read_to_string(home.join("hearth.lock"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Stop the daemon when the TUI owns it.
pub fn stop(home: &Path, ownership: &Ownership) -> Result<(), String> {
    if matches!(ownership, Ownership::ServiceManaged) {
        hearth_runtime::service::stop().map_err(|error| error.to_string())?;
        return wait_after_stop(home);
    }
    if probe(home).is_err() && lock_available(home) {
        return Ok(());
    }
    let pid = match ownership {
        Ownership::Spawned(pid) => lock_pid(home).unwrap_or(*pid),
        Ownership::Attached => lock_pid(home)
            .ok_or_else(|| "Hearth is running but its pid could not be read.".to_owned())?,
        Ownership::ServiceManaged => unreachable!(),
    };
    // The lock records the live daemon even after a restart.
    signal_stop(pid)?;
    wait_after_stop(home)
}

/// Restart a daemon the TUI owns.
pub fn restart(
    home: &Path,
    ownership: &Ownership,
    start: &str,
) -> Result<(Ownership, Value), String> {
    stop(home, ownership)?;
    ensure(home, start)
}

/// Apply a start mode and return the unit state.
pub fn apply_start_mode(home: &Path, start: &str) -> Result<(), String> {
    hearth_runtime::service::apply_start_mode(home, start)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn stop_on_exit(_home: &Path, ownership: &Ownership, keep_running: bool) {
    if keep_running {
        return;
    }
    if let Ownership::Spawned(pid) = ownership {
        let _ = signal_stop(*pid);
    }
}

fn wait_after_stop(home: &Path) -> Result<(), String> {
    if wait_for_stop(home) {
        Ok(())
    } else {
        Err("The daemon did not stop in time.".to_owned())
    }
}

fn signal_stop(pid: u32) -> Result<(), String> {
    let status = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|error| format!("could not signal the daemon: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the daemon did not accept the stop signal".to_owned())
    }
}

fn wait_for_status(home: &Path) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(status) = probe(home) {
            return Ok(status);
        }
        sleep(Duration::from_millis(100));
    }
    Err(format!(
        "The daemon did not start. Check {}.",
        home.join("logs/hearth.log").display()
    ))
}

fn wait_for_stop(home: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if probe(home).is_err() && lock_available(home) {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

fn lock_available(home: &Path) -> bool {
    let path = home.join("hearth.lock");
    let Ok(file) = std::fs::File::open(path) else {
        return true;
    };
    !matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

fn wait_for_existing_daemon(home: &Path) -> Option<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !lock_available(home) {
        if let Ok(status) = probe(home) {
            return Some(status);
        }
        if Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(100));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::process::Command;
    use std::sync::Mutex;

    /// `flock` is held by the open file description, so a child forked while a
    /// lock is held keeps it alive until it execs. Tests that fork and tests
    /// that assert on lock state must therefore never overlap.
    static FORK_GUARD: Mutex<()> = Mutex::new(());

    fn fork_guard() -> std::sync::MutexGuard<'static, ()> {
        FORK_GUARD.lock().unwrap_or_else(|error| error.into_inner())
    }

    #[test]
    fn lock_available_tracks_the_writer_lock() {
        let _guard = fork_guard();
        let home = tempfile::TempDir::new().expect("temp home");
        let path = home.path().join("hearth.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .expect("lock file");
        file.try_lock().expect("lock");
        assert!(!lock_available(home.path()));
        drop(file);
        assert!(lock_available(home.path()));
    }

    #[test]
    fn stop_on_exit_signals_without_waiting_for_the_lock() {
        let _guard = fork_guard();
        let home = tempfile::TempDir::new().expect("temp home");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(home.path().join("hearth.lock"))
            .expect("lock file");
        lock.try_lock().expect("lock");
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("child process");

        let started = Instant::now();
        stop_on_exit(home.path(), &Ownership::Spawned(child.id()), false);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "exit waited for the daemon"
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if child.try_wait().expect("poll child").is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!("the signaled process did not exit");
            }
            sleep(Duration::from_millis(20));
        }
    }
}
