use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use hearth_config::{ConfigError, Settings};
use hearth_core::{IdGen, UuidV7};
use hearth_model::{
    ChatMessage, ChatRequest, ContentBlock, OpenAiCompatibleProvider, Provider, ProviderError, Role,
};
use hearth_telegram::{TelegramClient, TelegramError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DoctorError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Storage(#[from] hearth_storage::StorageError),
    #[error("database not found at {0}; run `hearth` to set up the home")]
    DatabaseMissing(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Telegram(#[from] TelegramError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// Check the home folder and write the findings to a writer.
pub fn doctor_with(home: &Path, live: bool, out: &mut impl Write) -> Result<(), DoctorError> {
    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;

    let database = home.join("hearth.db");
    if !database.is_file() {
        return Err(DoctorError::DatabaseMissing(database));
    }
    let connection = hearth_storage::open_read_only(&database)?;
    hearth_storage::integrity_check(&connection)?;
    let version = hearth_storage::require_schema_version(&connection)?;

    let lock = lock_state(home)?;

    writeln!(out, "settings: ok")?;
    writeln!(out, "database: ok (schema {version})")?;
    writeln!(out, "lock: {lock}")?;

    if live {
        check_telegram(&settings, out)?;
        check_provider(&settings, out)?;
    }
    Ok(())
}

fn check_telegram(settings: &Settings, out: &mut impl Write) -> Result<(), DoctorError> {
    let Some(telegram) = settings.telegram.as_ref() else {
        writeln!(out, "telegram: not configured")?;
        return Ok(());
    };
    let Some(token) = telegram
        .bot_token
        .as_deref()
        .filter(|token| !token.is_empty())
    else {
        writeln!(out, "telegram: not configured")?;
        return Ok(());
    };
    let client = match telegram.base_url.as_deref() {
        Some(base_url) => TelegramClient::with_base_url(token, base_url)?,
        None => TelegramClient::new(token)?,
    };
    let me = client.get_me()?;
    let label = me
        .username
        .map(|username| format!("@{username}"))
        .unwrap_or_else(|| me.id.to_string());
    writeln!(out, "telegram: ok ({label})")?;
    Ok(())
}

fn check_provider(settings: &Settings, out: &mut impl Write) -> Result<(), DoctorError> {
    let models = settings.models.as_ref().ok_or_else(|| {
        DoctorError::Config(ConfigError::Invalid("models is required".to_owned()))
    })?;
    let provider = OpenAiCompatibleProvider::new(
        models.base_url.clone(),
        models.api_key.clone(),
        models
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        models.session_header.clone(),
    )?
    .with_reasoning_effort(models.thinking.clone());
    let request = ChatRequest {
        model: models.model.clone(),
        messages: vec![ChatMessage {
            role: Role::User,
            blocks: vec![ContentBlock::Text("Reply with OK.".to_owned())],
        }],
        tools: Vec::new(),
        temperature: None,
        max_tokens: None,
        session_id: Some(UuidV7.new_id()),
    };
    let stream = provider.stream(request)?;
    for event in stream {
        event?;
    }
    writeln!(out, "provider: ok ({})", models.model)?;
    Ok(())
}

fn lock_state(home: &Path) -> Result<String, DoctorError> {
    let path = home.join("hearth.lock");
    if !path.exists() {
        return Ok("not held".to_owned());
    }
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok("not held".to_owned()),
        Err(std::fs::TryLockError::WouldBlock) => {
            let pid = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            Ok(match pid {
                Some(pid) => format!("held (pid {pid})"),
                None => "held".to_owned(),
            })
        }
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}
