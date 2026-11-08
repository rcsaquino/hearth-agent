//! A small blocking client for the Telegram Bot API.

use std::io::Read;

use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::format::Entity;
use crate::types::{FileInfo, Update};

/// A Telegram API failure. Messages never include the token or request URL.
#[derive(Debug, Error)]
pub enum TelegramError {
    #[error("telegram {method} failed with {code:?}: {description}")]
    Api {
        method: String,
        code: Option<i64>,
        description: String,
        retry_after: Option<u64>,
    },
    #[error("telegram {method} returned an invalid response: {detail}")]
    Protocol { method: String, detail: String },
    #[error("telegram transport failed: {0}")]
    Transport(String),
    #[error("telegram file is larger than {limit} bytes")]
    TooLarge { limit: usize },
}

#[derive(Debug, Deserialize)]
struct Envelope {
    ok: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    error_code: Option<i64>,
    #[serde(default)]
    parameters: Option<Parameters>,
}

#[derive(Debug, Deserialize)]
struct Parameters {
    #[serde(default)]
    retry_after: Option<u64>,
}

/// A blocking Telegram Bot API client.
pub struct TelegramClient {
    token: String,
    base_url: String,
    http: reqwest::blocking::Client,
}

impl TelegramClient {
    /// Build a client against the public Bot API.
    pub fn new(token: impl Into<String>) -> Result<Self, TelegramError> {
        Self::with_base_url(token, "https://api.telegram.org")
    }

    /// Build a client against a custom Bot API base URL.
    pub fn with_base_url(
        token: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, TelegramError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|error| TelegramError::Transport(format!("client build failed: {error}")))?;
        Ok(Self {
            token: token.into(),
            base_url: base_url.into(),
            http,
        })
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, TelegramError> {
        let url = format!(
            "{}/bot{}/{}",
            self.base_url.trim_end_matches('/'),
            self.token,
            method
        );
        let response = self
            .http
            .post(url)
            .json(&params)
            .send()
            .map_err(transport_error)?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|_| TelegramError::Transport("could not read the response".to_owned()))?;
        let envelope: Envelope =
            serde_json::from_str(&body).map_err(|error| TelegramError::Protocol {
                method: method.to_owned(),
                detail: format!(
                    "status {status}, invalid JSON: {error}: {}",
                    body.chars().take(200).collect::<String>()
                ),
            })?;
        if !envelope.ok {
            return Err(TelegramError::Api {
                method: method.to_owned(),
                code: envelope.error_code.or_else(|| Some(status.as_u16() as i64)),
                description: envelope
                    .description
                    .unwrap_or_else(|| "unknown Telegram error".to_owned()),
                retry_after: envelope
                    .parameters
                    .and_then(|parameters| parameters.retry_after),
            });
        }
        Ok(envelope.result.unwrap_or(Value::Null))
    }

    /// Poll for updates.
    pub fn get_updates(
        &self,
        offset: i64,
        timeout: u32,
        allowed_updates: &[&str],
    ) -> Result<Vec<Update>, TelegramError> {
        let result = self.call(
            "getUpdates",
            json!({
                "offset": offset,
                "timeout": timeout,
                "allowed_updates": allowed_updates,
            }),
        )?;
        serde_json::from_value(result).map_err(|error| TelegramError::Protocol {
            method: "getUpdates".to_owned(),
            detail: format!("invalid update list: {error}"),
        })
    }

    /// Show a chat action such as typing.
    pub fn send_chat_action(&self, chat_id: i64, action: &str) -> Result<(), TelegramError> {
        self.call(
            "sendChatAction",
            json!({"chat_id": chat_id, "action": action}),
        )?;
        Ok(())
    }

    /// Send one native rich message with Markdown.
    pub fn send_rich_message(&self, chat_id: i64, markdown: &str) -> Result<(), TelegramError> {
        self.call(
            "sendRichMessage",
            json!({
                "chat_id": chat_id,
                "rich_message": {"markdown": markdown},
            }),
        )?;
        Ok(())
    }

    /// Send plain text with native entities.
    pub fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        entities: &[Entity],
    ) -> Result<(), TelegramError> {
        let mut params = json!({"chat_id": chat_id, "text": text});
        if !entities.is_empty() {
            params["entities"] =
                serde_json::to_value(entities).map_err(|error| TelegramError::Protocol {
                    method: "sendMessage".to_owned(),
                    detail: format!("entities could not be serialized: {error}"),
                })?;
        }
        self.call("sendMessage", params)?;
        Ok(())
    }

    /// Resolve one Telegram file descriptor.
    pub fn get_file(&self, file_id: &str) -> Result<FileInfo, TelegramError> {
        let result = self.call("getFile", json!({"file_id": file_id}))?;
        serde_json::from_value(result).map_err(|error| TelegramError::Protocol {
            method: "getFile".to_owned(),
            detail: format!("invalid file descriptor: {error}"),
        })
    }

    /// Download one Bot API file path with a hard byte cap.
    pub fn download_file(
        &self,
        file_path: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, TelegramError> {
        let base = format!(
            "{}/file/bot{}/",
            self.base_url.trim_end_matches('/'),
            self.token
        );
        let mut url =
            reqwest::Url::parse(&base).map_err(|error| TelegramError::Protocol {
                method: "download".to_owned(),
                detail: format!("invalid file base URL: {error}"),
            })?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| TelegramError::Protocol {
                    method: "download".to_owned(),
                    detail: "file path cannot be appended".to_owned(),
                })?;
            segments.pop_if_empty();
            for segment in file_path.split('/') {
                segments.push(segment);
            }
        }
        let response = self.http.get(url).send().map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(TelegramError::Api {
                method: "download".to_owned(),
                code: Some(status.as_u16() as i64),
                description: "file download failed".to_owned(),
                retry_after: None,
            });
        }
        let mut bytes = Vec::new();
        response
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                TelegramError::Transport(format!("could not read the file: {error}"))
            })?;
        if bytes.len() > max_bytes {
            return Err(TelegramError::TooLarge { limit: max_bytes });
        }
        Ok(bytes)
    }

    /// Acknowledge one message with an emoji reaction.
    pub fn set_message_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: &str,
    ) -> Result<(), TelegramError> {
        self.call(
            "setMessageReaction",
            json!({
                "chat_id": chat_id,
                "message_id": message_id,
                "reaction": [{"type": "emoji", "emoji": emoji}],
            }),
        )?;
        Ok(())
    }
}

fn transport_error(error: reqwest::Error) -> TelegramError {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else {
        "request"
    };
    TelegramError::Transport(format!("{kind} failure"))
}
