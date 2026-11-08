use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use hearth_config::SETTINGS_FILE;
use thiserror::Error;

/// Inputs for a non-interactive `hearth init`.
#[derive(Debug, Default)]
pub struct InitOptions {
    pub non_interactive: bool,
    pub telegram_token: Option<String>,
    pub allowed_user_ids: Vec<i64>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub agent_name: Option<String>,
    pub user_name: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("interactive setup is not implemented; use --non-interactive")]
    Interactive,
    #[error("{0} is required; pass {1}")]
    Missing(&'static str, &'static str),
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
pub fn init(home: &Path, options: InitOptions) -> Result<(), InitError> {
    if !options.non_interactive {
        return Err(InitError::Interactive);
    }

    let token = required(options.telegram_token, "telegram token", "--telegram-token")?;
    if options.allowed_user_ids.is_empty() {
        return Err(InitError::Missing("allowed user id", "--allowed-user-id"));
    }
    let base_url = required(options.base_url, "base URL", "--base-url")?;
    let api_key = required(options.api_key, "api key", "--api-key")?;
    let model = required(options.model, "model", "--model")?;
    let agent_name = options.agent_name.unwrap_or_else(|| "Hearth".to_owned());
    let user_name = options.user_name.unwrap_or_else(|| "User".to_owned());
    let timezone = options
        .timezone
        .or_else(system_timezone)
        .unwrap_or_else(|| "UTC".to_owned());

    fs::create_dir_all(home)?;
    for directory in [
        "workspace",
        "skills",
        "skills/learned",
        "media/inbound",
        "media/outbound",
        "spill",
        "logs",
        "backups",
    ] {
        fs::create_dir_all(home.join(directory))?;
    }

    let settings_path = home.join(SETTINGS_FILE);
    if settings_path.exists() {
        return Err(InitError::Exists(settings_path));
    }

    let mut models = serde_json::json!({
        "kind": "openai-compatible",
        "base_url": base_url,
        "api_key": api_key,
        "model": model,
    });
    if base_url.contains("opencode-go") || base_url.contains("opencode.ai/inference/go") {
        models["session_header"] = serde_json::json!("x-opencode-session");
    }
    let value = serde_json::json!({
        "identity": {
            "agent_name": agent_name,
            "user_name": user_name,
            "timezone": timezone,
        },
        "telegram": {
            "bot_token": token,
            "allowed_user_ids": options.allowed_user_ids,
        },
        "models": models,
    });
    let text = serde_json::to_string_pretty(&value).expect("settings are JSON values");

    let mut file = open_settings(&settings_path)?;
    file.write_all(text.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;

    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;

    let database = home.join("hearth.db");
    hearth_storage::open(&database)?;

    println!("created {}", home.display());
    println!("settings: {} (mode 0600)", settings_path.display());
    println!(
        "database: {} (schema {})",
        database.display(),
        hearth_storage::SCHEMA_VERSION
    );
    println!("next: hearth doctor --home {}", home.display());
    Ok(())
}

fn required(
    value: Option<String>,
    label: &'static str,
    flag: &'static str,
) -> Result<String, InitError> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(InitError::Missing(label, flag)),
    }
}

fn system_timezone() -> Option<String> {
    std::env::var("TZ").ok().filter(|value| !value.is_empty())
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
