//! Hearth settings: parsing, environment interpolation, and validation.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

mod settings;
mod validate;

pub use settings::{
    Agent, Compaction, Identity, ImageService, Logging, Media, Memory, Models, SearchProvider,
    Settings, SttService, Tasks, Telegram, TtsService, Web, WebSearch,
};

/// The name of the settings file inside the home folder.
pub const SETTINGS_FILE: &str = "settings.json";

/// A settings problem with a single-sentence, user-facing explanation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("settings.json not found at {0}; run hearth init")]
    Missing(PathBuf),
    #[error("settings.json could not be read: {0}")]
    Read(#[from] std::io::Error),
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
    let settings: Settings = serde_json::from_value(value)?;
    settings.validate()?;
    Ok(settings)
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
