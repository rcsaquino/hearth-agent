//! The systemd user unit for the daemon.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// A service installation failure.
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("a different unit already exists at {0}; pass --force")]
    DifferentUnit(PathBuf),
    #[error("systemctl failed: {0}")]
    Command(String),
    #[error("{0}")]
    Config(String),
}

/// The result of installing the unit.
#[derive(Debug, Clone)]
pub struct ServiceReport {
    pub unit: PathBuf,
}

/// The current unit state.
#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub unit: PathBuf,
    pub installed: bool,
}

/// Write and enable the user unit for the current binary and home.
pub fn install(home: &Path, force: bool) -> Result<ServiceReport, ServiceError> {
    let directory = config_dir()?;
    install_at(home, &directory, force, true)
}

/// Write the unit into a chosen systemd user directory.
///
/// `enable` is false in tests so no host service is touched.
pub fn install_at(
    home: &Path,
    config_dir: &Path,
    force: bool,
    enable: bool,
) -> Result<ServiceReport, ServiceError> {
    fs::create_dir_all(config_dir)?;
    let unit = config_dir.join("hearth.service");
    let content = unit_content(home)?;
    if let Ok(existing) = fs::read_to_string(&unit)
        && existing != content
        && !force
    {
        return Err(ServiceError::DifferentUnit(unit));
    }
    fs::write(&unit, content)?;
    if enable {
        run_systemctl(&["daemon-reload"])?;
        run_systemctl(&["enable", "hearth.service"])?;
    }
    Ok(ServiceReport { unit })
}

/// Report whether the unit is installed.
pub fn status(home: &Path) -> Result<ServiceStatus, ServiceError> {
    let _ = home;
    let directory = config_dir()?;
    status_at(&directory)
}

/// Report the state of a unit in a chosen directory.
pub fn status_at(config_dir: &Path) -> Result<ServiceStatus, ServiceError> {
    let unit = config_dir.join("hearth.service");
    Ok(ServiceStatus {
        installed: unit.is_file(),
        unit,
    })
}

fn unit_content(home: &Path) -> Result<String, ServiceError> {
    let executable = std::env::current_exe()?;
    Ok(format!(
        "[Unit]\n\
         Description=Hearth personal agent\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} daemon --home {}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        executable.display(),
        home.display()
    ))
}

fn run_systemctl(args: &[&str]) -> Result<(), ServiceError> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .map_err(ServiceError::Io)?;
    if status.success() {
        Ok(())
    } else {
        Err(ServiceError::Command(format!(
            "systemctl --user {}",
            args.join(" ")
        )))
    }
}

fn config_dir() -> Result<PathBuf, ServiceError> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| ServiceError::Config("HOME is not set; pass an XDG config directory".into()))?;
    Ok(base.join("systemd/user"))
}
