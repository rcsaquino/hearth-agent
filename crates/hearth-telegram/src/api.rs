//! A small blocking client for the Telegram Bot API.

use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::format::Entity;
use crate::types::Update;

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
