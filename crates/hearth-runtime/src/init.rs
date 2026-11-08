use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use hearth_config::SETTINGS_FILE;
use serde_json::{Value, json};
use thiserror::Error;

/// Inputs for `hearth init`.
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
    #[error("{0} is required; pass {1}")]
    Missing(&'static str, &'static str),
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
pub fn init(home: &Path, options: InitOptions) -> Result<(), InitError> {
    let mut reader = std::io::stdin().lock();
    let mut writer = std::io::stdout().lock();
    init_with_io(home, options, &mut reader, &mut writer)
}

/// Create a home folder using an injected reader and writer.
pub fn init_with_io<R: BufRead, W: Write>(
    home: &Path,
    mut options: InitOptions,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), InitError> {
    let mut media: Option<Value> = None;
    if !options.non_interactive {
        if options
            .telegram_token
            .as_deref()
            .is_none_or(|value| value.is_empty())
        {
            options.telegram_token = Some(prompt(reader, writer, "Telegram bot token: ")?);
        }
        if options.allowed_user_ids.is_empty() {
            let value = prompt(reader, writer, "Allowed Telegram user id: ")?;
            let id = value.parse::<i64>().map_err(|_| {
                InitError::Invalid(format!("allowed user id must be a number: {value}"))
            })?;
            options.allowed_user_ids = vec![id];
        }
        if options
            .base_url
            .as_deref()
            .is_none_or(|value| value.is_empty())
        {
            options.base_url = Some(prompt(reader, writer, "Provider base URL: ")?);
        }
        if options
            .api_key
            .as_deref()
            .is_none_or(|value| value.is_empty())
        {
            options.api_key = Some(prompt(reader, writer, "Provider API key: ")?);
        }
        if options.model.as_deref().is_none_or(|value| value.is_empty()) {
            options.model = Some(prompt(reader, writer, "Model: ")?);
        }
        if options.agent_name.is_none() {
            let value = prompt(reader, writer, "Agent name [Hearth]: ")?;
            if !value.is_empty() {
                options.agent_name = Some(value);
            }
        }
        if options.user_name.is_none() {
            let value = prompt(reader, writer, "User name [User]: ")?;
            if !value.is_empty() {
                options.user_name = Some(value);
            }
        }
        if options.timezone.is_none() {
            let value = prompt(reader, writer, "Timezone [system]: ")?;
            if !value.is_empty() {
                options.timezone = Some(value);
            }
        }
        media = prompt_media(reader, writer)?;
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

    let soul_path = home.join("SOUL.md");
    if !soul_path.exists() {
        let mut soul = open_settings(&soul_path)?;
        soul.write_all(hearth_core::default_soul(&agent_name, &user_name).as_bytes())?;
        soul.flush()?;
    }

    let mut models = json!({
        "kind": "openai-compatible",
        "base_url": base_url,
        "api_key": api_key,
        "model": model,
    });
    if base_url.contains("opencode-go") || base_url.contains("opencode.ai/inference/go") {
        models["session_header"] = json!("x-opencode-session");
    }
    let mut value = json!({
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
    if let Some(media) = media {
        value["media"] = media;
    }
    let text = serde_json::to_string_pretty(&value).expect("settings are JSON values");

    let mut file = open_settings(&settings_path)?;
    file.write_all(text.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;

    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;

    let database = home.join("hearth.db");
    hearth_storage::open(&database)?;

    writeln!(writer, "created {}", home.display())?;
    writeln!(
        writer,
        "settings: {} (mode 0600)",
        settings_path.display()
    )?;
    writeln!(
        writer,
        "database: {} (schema {})",
        database.display(),
        hearth_storage::SCHEMA_VERSION
    )?;
    writeln!(
        writer,
        "next: hearth doctor --live --home {}",
        home.display()
    )?;
    Ok(())
}

fn prompt<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> Result<String, InitError> {
    write!(writer, "{label}")?;
    writer.flush()?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

fn yes<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> Result<bool, InitError> {
    let answer = prompt(reader, writer, label)?;
    Ok(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn prompt_media<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> Result<Option<Value>, InitError> {
    let mut media = serde_json::Map::new();
    if yes(reader, writer, "Configure image generation? [y/N] ")? {
        let api_key = prompt(reader, writer, "Image API key: ")?;
        let model = prompt(
            reader,
            writer,
            "Image model [openai/gpt-image-2.6-sunburst]: ",
        )?;
        let model = if model.is_empty() {
            "openai/gpt-image-2.6-sunburst".to_owned()
        } else {
            model
        };
        media.insert(
            "image".to_owned(),
            json!({"kind": "openrouter-images", "api_key": api_key, "model": model}),
        );
    }
    if yes(reader, writer, "Configure transcription? [y/N] ")? {
        let api_key = prompt(reader, writer, "Transcription API key: ")?;
        let model = prompt(
            reader,
            writer,
            "Transcription model [whisper-large-v3-turbo]: ",
        )?;
        let model = if model.is_empty() {
            "whisper-large-v3-turbo".to_owned()
        } else {
            model
        };
        media.insert(
            "stt".to_owned(),
            json!({"kind": "groq", "api_key": api_key, "model": model}),
        );
    }
    if yes(reader, writer, "Configure speech? [y/N] ")? {
        let api_key = prompt(reader, writer, "Speech API key: ")?;
        let voice_id = prompt(reader, writer, "Speech voice id: ")?;
        let model = prompt(reader, writer, "Speech model [eleven_v4]: ")?;
        let model = if model.is_empty() {
            "eleven_v4".to_owned()
        } else {
            model
        };
        media.insert(
            "tts".to_owned(),
            json!({
                "kind": "elevenlabs",
                "api_key": api_key,
                "voice_id": voice_id,
                "model": model,
                "output_format": "opus_48000_64",
            }),
        );
    }
    Ok(if media.is_empty() {
        None
    } else {
        Some(Value::Object(media))
    })
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
