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
    #[error("a different unit already exists at {0}")]
    DifferentUnit(PathBuf),
    #[error("command failed: {0}")]
    Command(String),
    #[error("{0}")]
    Config(String),
}

/// The result of installing the unit.
#[derive(Debug, Clone)]
pub struct ServiceReport {
    pub unit: PathBuf,
    pub created: bool,
}

/// The current unit state.
#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub unit: PathBuf,
    pub installed: bool,
}

/// The result of removing the unit.
#[derive(Debug, Clone)]
pub struct ServiceRemoval {
    pub unit: PathBuf,
    pub removed: bool,
}

/// The result of applying a daemon start mode.
#[derive(Debug, Clone)]
pub struct StartModeReport {
    pub start: String,
    pub unit: PathBuf,
    pub installed: bool,
    pub linger: bool,
    pub repaired: bool,
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
    let existing = fs::read_to_string(&unit).ok();
    if let Some(existing) = existing.as_deref()
        && existing != content
        && !force
    {
        return Err(ServiceError::DifferentUnit(unit));
    }
    let created = existing.as_deref() != Some(content.as_str());
    fs::write(&unit, content)?;
    link_unit(config_dir, &unit)?;
    if enable {
        run_systemctl(&["daemon-reload"])?;
        run_systemctl(&["enable", "hearth.service"])?;
    }
    Ok(ServiceReport { unit, created })
}

/// Report whether the unit is installed for this home.
pub fn status(home: &Path) -> Result<ServiceStatus, ServiceError> {
    let directory = config_dir()?;
    status_at(&directory, home)
}

/// Report the state of a unit in a chosen directory.
///
/// A unit that names another home is not installed for this one: starting it
/// would run a daemon against the wrong record.
pub fn status_at(config_dir: &Path, home: &Path) -> Result<ServiceStatus, ServiceError> {
    let unit = config_dir.join("hearth.service");
    let installed = unit.is_file() && unit_home(&unit).is_some_and(|named| named == home);
    Ok(ServiceStatus { unit, installed })
}

/// Remove the unit from a chosen directory.
///
/// `systemctl` is false in tests so no host service is touched.
pub fn uninstall_at(config_dir: &Path, systemctl: bool) -> Result<ServiceRemoval, ServiceError> {
    let unit = config_dir.join("hearth.service");
    if systemctl {
        let _ = run_systemctl(&["stop", "hearth.service"]);
        let _ = run_systemctl(&["disable", "hearth.service"]);
    }
    let removed = match fs::remove_file(&unit) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(ServiceError::Io(error)),
    };
    match fs::remove_file(wants_link(config_dir)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(ServiceError::Io(error)),
    }
    if systemctl {
        run_systemctl(&["daemon-reload"])?;
    }
    Ok(ServiceRemoval { unit, removed })
}

/// Start the installed user unit.
pub fn start() -> Result<(), ServiceError> {
    run_systemctl(&["start", "hearth.service"])
}

/// Stop the installed user unit.
pub fn stop() -> Result<(), ServiceError> {
    run_systemctl(&["stop", "hearth.service"])
}

/// Restart the installed user unit so it re-reads its home and settings.
pub fn restart() -> Result<(), ServiceError> {
    run_systemctl(&["restart", "hearth.service"])
}

/// Apply a daemon start mode: `tui`, `login`, or `startup`.
pub fn apply_start_mode(home: &Path, start: &str) -> Result<StartModeReport, ServiceError> {
    let directory = config_dir()?;
    apply_start_mode_at(home, &directory, start, true)
}

/// Apply a start mode in a chosen directory.
///
/// `run` is false in tests so systemctl and loginctl never touch the host.
pub fn apply_start_mode_at(
    home: &Path,
    config_dir: &Path,
    start: &str,
    run: bool,
) -> Result<StartModeReport, ServiceError> {
    let unit = config_dir.join("hearth.service");
    match start {
        "tui" => {
            // A dangling link outlives the unit it named, so the unit's own
            // presence is not enough to decide whether there is anything to do.
            let present = unit.is_file() || fs::symlink_metadata(wants_link(config_dir)).is_ok();
            if present {
                uninstall_at(config_dir, run)?;
            }
            if run {
                let _ = disable_linger();
            }
            Ok(StartModeReport {
                start: start.to_owned(),
                unit,
                installed: false,
                linger: false,
                repaired: present,
            })
        }
        "login" => {
            let linked = link_points_at(config_dir, &unit);
            let report = install_at(home, config_dir, true, run)?;
            if run {
                let _ = disable_linger();
            }
            Ok(StartModeReport {
                start: start.to_owned(),
                unit,
                installed: true,
                linger: false,
                repaired: report.created || !linked,
            })
        }
        "startup" => {
            let linked = link_points_at(config_dir, &unit);
            let report = install_at(home, config_dir, true, run)?;
            let mut repaired = report.created || !linked;
            if run {
                repaired |= !linger_present()?;
                enable_linger()?;
            }
            Ok(StartModeReport {
                start: start.to_owned(),
                unit,
                installed: true,
                linger: true,
                repaired,
            })
        }
        other => Err(ServiceError::Config(format!(
            "unknown daemon start mode: {other}"
        ))),
    }
}

/// Restore the unit and its enable link from the configured start mode.
///
/// The pass is additive: it creates what is missing and never rewrites an
/// existing unit or removes anything.
pub fn repair(home: &Path) -> Result<Option<StartModeReport>, ServiceError> {
    repair_at(home, &config_dir()?, true)
}

/// Restore the unit and its enable link in a chosen directory.
///
/// The files are the whole state: systemd loads a unit on demand and reads the
/// enable link when `default.target` comes up, so a repair must not depend on a
/// reachable user manager. `run` is false in tests so loginctl never touches the
/// host.
pub fn repair_at(
    home: &Path,
    config_dir: &Path,
    run: bool,
) -> Result<Option<StartModeReport>, ServiceError> {
    let settings =
        hearth_config::load(home).map_err(|error| ServiceError::Config(error.to_string()))?;
    let start = settings.daemon.start;
    let unit = config_dir.join("hearth.service");
    let repaired = match start.as_str() {
        "tui" => return Ok(None),
        "login" | "startup" => {
            let mut repaired = if unit.is_file() {
                if unit_home(&unit).is_some_and(|named| named == home) {
                    link_unit(config_dir, &unit)?
                } else {
                    return Ok(None);
                }
            } else {
                install_at(home, config_dir, false, false)?.created
            };
            if start == "startup" && run && !linger_present()? {
                enable_linger()?;
                repaired = true;
            }
            repaired
        }
        other => {
            return Err(ServiceError::Config(format!(
                "unknown daemon start mode: {other}"
            )));
        }
    };
    let linger = start == "startup";
    Ok(Some(StartModeReport {
        start,
        unit,
        installed: true,
        linger,
        repaired,
    }))
}

fn wants_link(config_dir: &Path) -> PathBuf {
    config_dir
        .join("default.target.wants")
        .join("hearth.service")
}

fn link_points_at(config_dir: &Path, unit: &Path) -> bool {
    fs::read_link(wants_link(config_dir)).is_ok_and(|target| target == unit)
}

fn link_unit(config_dir: &Path, unit: &Path) -> Result<bool, ServiceError> {
    let link = wants_link(config_dir);
    if let Some(directory) = link.parent() {
        fs::create_dir_all(directory)?;
    }
    if fs::read_link(&link).is_ok_and(|target| target == unit) {
        return Ok(false);
    }
    match fs::remove_file(&link) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(ServiceError::Io(error)),
    }
    std::os::unix::fs::symlink(unit, &link)?;
    Ok(true)
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
         Environment=\"HEARTH_DAEMON=1\"\n\
         Environment=\"HEARTH_HOME={}\"\n\
         ExecStart=\"{}\"\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        escape(&home.to_string_lossy()),
        escape(&executable.to_string_lossy())
    ))
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
}

/// Expand the specifiers a user unit may name in a path.
///
/// The packaged unit names `%h/.hearth`; systemd expands `%h` to the user's home.
/// Without that expansion the packaged unit looks like it belongs to another home
/// and the product reports it as not installed.
fn expand_specifiers(value: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut expanded = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        match characters.peek().copied() {
            Some('%') => {
                characters.next();
                expanded.push('%');
            }
            Some('h') => {
                characters.next();
                expanded.push_str(&home);
            }
            _ => expanded.push('%'),
        }
    }
    expanded
}

/// The home a unit names, or `None` when it names none or cannot be read.
fn unit_home(unit: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(unit).ok()?;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("Environment=") else {
            continue;
        };
        let rest = rest.trim();
        let value = rest
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(rest);
        // A quoted value may hold one assignment whose path contains spaces, or
        // several space-separated assignments.
        if let Some(home) = value.strip_prefix("HEARTH_HOME=")
            && !home.is_empty()
        {
            return Some(PathBuf::from(expand_specifiers(home)));
        }
        for assignment in value.split_whitespace() {
            if let Some(home) = assignment.strip_prefix("HEARTH_HOME=")
                && !home.is_empty()
            {
                return Some(PathBuf::from(expand_specifiers(home)));
            }
        }
    }
    None
}

fn linger_present() -> Result<bool, ServiceError> {
    let user = current_user()?;
    Ok(Path::new("/var/lib/systemd/linger").join(user).is_file())
}

fn enable_linger() -> Result<(), ServiceError> {
    let user = current_user()?;
    run_loginctl(&["enable-linger", &user])
}

fn disable_linger() -> Result<(), ServiceError> {
    let user = current_user()?;
    run_loginctl(&["disable-linger", &user])
}

fn current_user() -> Result<String, ServiceError> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map_err(|_| ServiceError::Config("USER is not set; cannot change linger".to_owned()))
}

fn run_systemctl(args: &[&str]) -> Result<(), ServiceError> {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    run_command("systemctl", &full)
}

fn run_loginctl(args: &[&str]) -> Result<(), ServiceError> {
    run_command("loginctl", args)
}

fn run_command(program: &str, args: &[&str]) -> Result<(), ServiceError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(ServiceError::Io)?;
    if output.status.success() {
        return Ok(());
    }
    let command = format!("{program} {}", args.join(" "));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.lines().next().unwrap_or_default().trim();
    Err(ServiceError::Command(if detail.is_empty() {
        command
    } else {
        format!("{command}: {detail}")
    }))
}

fn config_dir() -> Result<PathBuf, ServiceError> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| {
            ServiceError::Config("HOME is not set; pass an XDG config directory".into())
        })?;
    Ok(base.join("systemd/user"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_home_a_unit_names() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let unit = directory.path().join("hearth.service");
        fs::write(
            &unit,
            "[Service]\nEnvironment=\"HEARTH_DAEMON=1\"\n\
             Environment=\"HEARTH_HOME=/home/USER_NAME/.hearth\"\n",
        )
        .expect("unit");
        assert_eq!(
            unit_home(&unit),
            Some(PathBuf::from("/home/USER_NAME/.hearth"))
        );
    }

    #[test]
    fn reads_a_home_from_a_combined_environment_line() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let unit = directory.path().join("hearth.service");
        fs::write(
            &unit,
            "[Service]\nEnvironment=\"HEARTH_DAEMON=1 HEARTH_HOME=/srv/hearth\"\n",
        )
        .expect("unit");
        assert_eq!(unit_home(&unit), Some(PathBuf::from("/srv/hearth")));
    }

    #[test]
    fn reads_a_home_that_contains_spaces() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let unit = directory.path().join("hearth.service");
        fs::write(
            &unit,
            "[Service]\nEnvironment=\"HEARTH_HOME=/home/USER_NAME/My Files/.hearth\"\n",
        )
        .expect("unit");
        assert_eq!(
            unit_home(&unit),
            Some(PathBuf::from("/home/USER_NAME/My Files/.hearth"))
        );
    }

    #[test]
    fn a_unit_without_a_home_names_none() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let unit = directory.path().join("hearth.service");
        fs::write(&unit, "[Service]\nEnvironment=\"HEARTH_DAEMON=1\"\n").expect("unit");
        assert_eq!(unit_home(&unit), None);
        assert_eq!(unit_home(&directory.path().join("missing.service")), None);
    }
}
