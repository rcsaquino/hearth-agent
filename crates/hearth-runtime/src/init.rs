use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use hearth_config::{Daemon, SETTINGS_FILE};
use serde_json::{Value, json};
use thiserror::Error;

/// Inputs for creating a home.
#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub telegram_token: String,
    pub allowed_user_ids: Vec<i64>,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub thinking: String,
    pub agent_name: String,
    pub timezone: Option<String>,
    pub media: Option<Value>,
    pub context_window: Option<u64>,
    pub daemon: Daemon,
}

/// What init created.
#[derive(Debug, Clone)]
pub struct InitReport {
    pub home: PathBuf,
    pub settings: PathBuf,
    pub database: PathBuf,
    pub schema_version: i64,
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("{0}")]
    Invalid(String),
    #[error("settings.json already exists at {0}; refusing to overwrite")]
    Exists(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Config(#[from] hearth_config::ConfigError),
    #[error(transparent)]
    Storage(#[from] hearth_storage::StorageError),
}

/// Create a home folder, settings file, directory layout, and database.
pub fn init(home: &Path, options: InitOptions) -> Result<InitReport, InitError> {
    validate(&options)?;

    // The settings file is the front door: refuse before writing anything else.
    let settings_path = home.join(SETTINGS_FILE);
    if settings_path.exists() {
        return Err(InitError::Exists(settings_path));
    }
    crate::home::scaffold(home, &options.agent_name)?;

    let timezone = options
        .timezone
        .clone()
        .or_else(system_timezone)
        .unwrap_or_else(|| "UTC".to_owned());
    let mut models = json!({
        "kind": "openai-compatible",
        "base_url": options.base_url,
        "api_key": options.api_key,
        "model": options.model,
        "thinking": options.thinking,
    });
    if let Some(window) = options.context_window {
        models["context_window"] = json!(window);
    }
    if models["base_url"]
        .as_str()
        .is_some_and(|url| url.contains("opencode-go") || url.contains("opencode.ai/inference/go"))
    {
        models["session_header"] = json!("x-opencode-session");
    }
    let mut value = json!({
        "identity": {
            "agent_name": options.agent_name,
            "timezone": timezone,
        },
        "telegram": {
            "bot_token": options.telegram_token,
            "allowed_user_ids": options.allowed_user_ids,
        },
        "models": models,
        "daemon": {
            "start": options.daemon.start,
            "keep_running": options.daemon.keep_running,
        },
    });
    if let Some(media) = options.media {
        value["media"] = media;
    }
    let value = hearth_config::compact_settings(&value);
    let text = serde_json::to_string_pretty(&value).expect("settings are JSON values");

    let mut file = open_settings(&settings_path)?;
    file.write_all(text.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;

    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;

    let database = home.join("hearth.db");
    hearth_storage::open(&database)?;

    Ok(InitReport {
        home: home.to_path_buf(),
        settings: settings_path,
        database,
        schema_version: hearth_storage::SCHEMA_VERSION,
    })
}

fn validate(options: &InitOptions) -> Result<(), InitError> {
    if options.telegram_token.trim().is_empty() {
        return Err(invalid("telegram bot token is required"));
    }
    if options.allowed_user_ids.is_empty() {
        return Err(invalid("at least one Telegram user id is required"));
    }
    if !(options.base_url.starts_with("https://") || options.base_url.starts_with("http://")) {
        return Err(invalid(
            "provider base URL must start with http:// or https://",
        ));
    }
    if options.api_key.trim().is_empty() {
        return Err(invalid("provider API key is required"));
    }
    if options.model.trim().is_empty() {
        return Err(invalid("model is required"));
    }
    if options.agent_name.trim().is_empty() {
        return Err(invalid("agent name is required"));
    }
    if !matches!(
        options.thinking.as_str(),
        "default" | "off" | "low" | "medium" | "high" | "xhigh" | "max"
    ) {
        return Err(invalid(
            "thinking must be one of default, off, low, medium, high, xhigh, or max",
        ));
    }
    if !matches!(options.daemon.start.as_str(), "tui" | "login" | "startup") {
        return Err(invalid("daemon start must be tui, login, or startup"));
    }
    Ok(())
}

fn invalid(message: &str) -> InitError {
    InitError::Invalid(message.to_owned())
}

fn system_timezone() -> Option<String> {
    if let Some(value) = std::env::var("TZ").ok().filter(|value| !value.is_empty()) {
        return Some(value);
    }
    if let Some(zone) = timezone_from_localtime(Path::new("/etc/localtime")) {
        return Some(zone);
    }
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn timezone_from_target(target: &Path) -> Option<String> {
    let target = target.to_str()?;
    let (_, zone) = target.rsplit_once("/zoneinfo/")?;
    let zone = zone.trim_matches('/');
    if zone.is_empty() {
        None
    } else {
        Some(zone.to_owned())
    }
}

fn timezone_from_localtime(path: &Path) -> Option<String> {
    let target = std::fs::read_link(path).ok()?;
    timezone_from_target(&target)
}

#[cfg(unix)]
fn open_settings(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_settings(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_zone_from_an_absolute_zoneinfo_target() {
        assert_eq!(
            timezone_from_target(Path::new("/usr/share/zoneinfo/Asia/Manila")),
            Some("Asia/Manila".to_owned())
        );
    }

    #[test]
    fn extracts_a_zone_from_a_relative_zoneinfo_target() {
        assert_eq!(
            timezone_from_target(Path::new("../usr/share/zoneinfo/Etc/UTC")),
            Some("Etc/UTC".to_owned())
        );
    }

    #[test]
    fn ignores_a_target_outside_zoneinfo() {
        assert_eq!(timezone_from_target(Path::new("/etc/localtime")), None);
    }

    #[test]
    fn ignores_an_empty_target() {
        assert_eq!(timezone_from_target(Path::new("")), None);
    }

    #[cfg(unix)]
    #[test]
    fn reads_a_zone_from_a_localtime_symlink() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let link = directory.path().join("localtime");
        std::os::unix::fs::symlink("/usr/share/zoneinfo/Asia/Manila", &link).expect("symlink");
        assert_eq!(
            timezone_from_localtime(&link),
            Some("Asia/Manila".to_owned())
        );
    }
}
