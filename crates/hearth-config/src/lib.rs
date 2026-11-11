//! Hearth settings: parsing, environment interpolation, and validation.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

mod settings;
mod validate;

pub use settings::{
    Agent, Compaction, Daemon, FetchProvider, Identity, ImageService, Logging, Media, Memory,
    Models, SearchProvider, Settings, SttService, Tasks, Telegram, TtsService, Web, WebFetch,
    WebSearch, compact_settings, default_settings_value,
};

/// The name of the settings file inside the home folder.
pub const SETTINGS_FILE: &str = "settings.json";

/// A settings problem with a single-sentence, user-facing explanation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("settings.json not found at {0}; run `hearth` to set up the home")]
    Missing(PathBuf),
    #[error("settings.json could not be read: {0}")]
    Read(#[from] std::io::Error),
    #[error("settings.json could not be written: {0}")]
    Write(std::io::Error),
    #[error("settings.json is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("settings.json has mode {mode:o}; run chmod 600 {path}")]
    Mode { path: PathBuf, mode: u32 },
    #[error("{0}")]
    Invalid(String),
}

/// Load the settings file in `home`.
pub fn load(home: &Path) -> Result<Settings, ConfigError> {
    let path = home.join(SETTINGS_FILE);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConfigError::Missing(path));
        }
        Err(error) => return Err(ConfigError::Read(error)),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mode = metadata.mode() & 0o777;
        if mode != 0o600 {
            return Err(ConfigError::Mode { path, mode });
        }
    }
    let text = fs::read_to_string(&path)?;
    let text = if text.trim().is_empty() {
        "{}".to_owned()
    } else {
        text
    };
    let mut value: Value = serde_json::from_str(&text)?;
    expand_value(&mut value);
    strip_legacy(&mut value);
    let settings: Settings = serde_json::from_value(value)?;
    settings.validate()?;
    Ok(settings)
}

/// Update the `daemon` section in place and omit values that match defaults.
///
/// The file is replaced atomically with mode 0600 and reloaded for validation.
pub fn save_daemon(home: &Path, daemon: &Daemon) -> Result<(), ConfigError> {
    let path = home.join(SETTINGS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConfigError::Missing(path));
        }
        Err(error) => return Err(ConfigError::Read(error)),
    };
    let mut value: Value = if text.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(&text)?
    };
    let object = value.as_object_mut().ok_or_else(|| {
        ConfigError::Invalid("settings.json must contain a JSON object".to_owned())
    })?;
    object.insert("daemon".to_owned(), serde_json::to_value(daemon)?);
    let value = compact_settings(&value);
    let mut output = serde_json::to_string_pretty(&value)?;
    output.push('\n');
    write_atomic(&path, output.as_bytes())?;
    load(home)?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(SETTINGS_FILE);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp = path.with_file_name(format!(".{name}.tmp.{}.{nanos}", std::process::id()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(ConfigError::Write)
}

/// Remove settings that no longer exist but old files may still contain.
fn strip_legacy(value: &mut Value) {
    if let Some(identity) = value.get_mut("identity").and_then(Value::as_object_mut) {
        identity.remove("soul_file");
    }
    if let Some(models) = value.get_mut("models").and_then(Value::as_object_mut) {
        models.remove("pricing");
    }
}

/// Replace every `${NAME}` with the value of environment variable `NAME`.
/// A missing variable is left as the literal reference so validation can name it.
fn expand_value(value: &mut Value) {
    match value {
        Value::String(text) => *text = expand_string(text),
        Value::Array(items) => items.iter_mut().for_each(expand_value),
        Value::Object(map) => map.values_mut().for_each(expand_value),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn expand_string(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => result.push_str(&value),
                    Err(_) => result.push_str(&rest[start..start + 2 + end + 1]),
                }
                rest = &after[end + 1..];
            }
            None => {
                result.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    result.push_str(rest);
    result
}

/// The first unresolved `${NAME}` reference in `value`, if any.
pub(crate) fn unresolved(value: &str) -> Option<String> {
    let start = value.find("${")?;
    let after = &value[start + 2..];
    let end = after.find('}')?;
    Some(after[..end].to_owned())
}
