//! A small blocking client for the Telegram Bot API.

use std::io::Read;
use std::path::{Path, PathBuf};

use reqwest::blocking::multipart::{Form, Part};
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

/// The media kinds accepted in one album.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Photo,
    Video,
    Document,
    Voice,
}

impl MediaKind {
    pub(crate) fn album_type(self) -> Option<&'static str> {
        match self {
            Self::Photo => Some("photo"),
            Self::Video => Some("video"),
            Self::Document | Self::Voice => None,
        }
    }
}

/// A blocking Telegram Bot API client.
#[derive(Clone)]
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
        let response = self
            .http
            .post(self.url(method))
            .json(&params)
            .send()
            .map_err(transport_error)?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|_| TelegramError::Transport("could not read the response".to_owned()))?;
        self.parse_envelope(method, status, &body)
    }

    fn call_multipart(&self, method: &str, form: Form) -> Result<Value, TelegramError> {
        let response = self
            .http
            .post(self.url(method))
            .multipart(form)
            .send()
            .map_err(transport_error)?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|_| TelegramError::Transport("could not read the response".to_owned()))?;
        self.parse_envelope(method, status, &body)
    }

    fn url(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            self.base_url.trim_end_matches('/'),
            self.token,
            method
        )
    }

    fn parse_envelope(
        &self,
        method: &str,
        status: reqwest::StatusCode,
        body: &str,
    ) -> Result<Value, TelegramError> {
        let envelope: Envelope =
            serde_json::from_str(body).map_err(|error| TelegramError::Protocol {
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
    pub fn send_chat_action(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        action: &str,
    ) -> Result<(), TelegramError> {
        let mut params = json!({"chat_id": chat_id, "action": action});
        if let Some(thread_id) = thread_id {
            params["message_thread_id"] = json!(thread_id);
        }
        self.call("sendChatAction", params)?;
        Ok(())
    }

    /// Send one native rich message with Markdown.
    pub fn send_rich_message(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        markdown: &str,
    ) -> Result<(), TelegramError> {
        let mut params = json!({
            "chat_id": chat_id,
            "rich_message": {"markdown": markdown},
        });
        if let Some(thread_id) = thread_id {
            params["message_thread_id"] = json!(thread_id);
        }
        self.call("sendRichMessage", params)?;
        Ok(())
    }

    /// Send plain text with native entities.
    pub fn send_message(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        text: &str,
        entities: &[Entity],
    ) -> Result<(), TelegramError> {
        let mut params = json!({"chat_id": chat_id, "text": text});
        if let Some(thread_id) = thread_id {
            params["message_thread_id"] = json!(thread_id);
        }
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

    /// Send one photo and return its message id.
    pub fn send_photo(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.send_file("sendPhoto", "photo", chat_id, thread_id, path, caption)
    }

    /// Send one document and return its message id.
    pub fn send_document(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.send_file("sendDocument", "document", chat_id, thread_id, path, caption)
    }

    /// Send one video and return its message id.
    pub fn send_video(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.send_file("sendVideo", "video", chat_id, thread_id, path, caption)
    }

    /// Send one voice note and return its message id.
    pub fn send_voice(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.send_file("sendVoice", "voice", chat_id, thread_id, path, caption)
    }

    fn send_file(
        &self,
        method: &str,
        field: &str,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        let mut form = Form::new().text("chat_id", chat_id.to_string());
        if let Some(thread_id) = thread_id {
            form = form.text("message_thread_id", thread_id.to_string());
        }
        if let Some(caption) = caption.filter(|value| !value.is_empty()) {
            form = form.text("caption", caption.to_owned());
        }
        let part = Part::file(path).map_err(|error| TelegramError::Protocol {
            method: method.to_owned(),
            detail: format!("file could not be opened: {error}"),
        })?;
        form = form.part(field.to_owned(), part);
        let result = self.call_multipart(method, form)?;
        result["message_id"]
            .as_i64()
            .ok_or_else(|| TelegramError::Protocol {
                method: method.to_owned(),
                detail: "response has no message id".to_owned(),
            })
    }

    /// Send two to ten photos or videos as one album.
    pub fn send_media_group(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        items: &[(MediaKind, PathBuf)],
        caption: Option<&str>,
    ) -> Result<Vec<i64>, TelegramError> {
        if items.is_empty() {
            return Err(TelegramError::Protocol {
                method: "sendMediaGroup".to_owned(),
                detail: "an album needs at least one item".to_owned(),
            });
        }
        let mut media = Vec::with_capacity(items.len());
        for (index, (kind, _)) in items.iter().enumerate() {
            let media_type = kind.album_type().ok_or_else(|| TelegramError::Protocol {
                method: "sendMediaGroup".to_owned(),
                detail: "only photos and videos can form an album".to_owned(),
            })?;
            let mut entry = json!({
                "type": media_type,
                "media": format!("attach://file{index}"),
            });
            if index == 0
                && let Some(caption) = caption.filter(|value| !value.is_empty())
            {
                entry["caption"] = Value::String(caption.to_owned());
            }
            media.push(entry);
        }
        let media_json =
            serde_json::to_string(&media).map_err(|error| TelegramError::Protocol {
                method: "sendMediaGroup".to_owned(),
                detail: format!("media list could not be serialized: {error}"),
            })?;
        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .text("media", media_json);
        if let Some(thread_id) = thread_id {
            form = form.text("message_thread_id", thread_id.to_string());
        }
        for (index, (_, path)) in items.iter().enumerate() {
            let part = Part::file(path).map_err(|error| TelegramError::Protocol {
                method: "sendMediaGroup".to_owned(),
                detail: format!("file could not be opened: {error}"),
            })?;
            form = form.part(format!("file{index}"), part);
        }
        let result = self.call_multipart("sendMediaGroup", form)?;
        let array = result.as_array().ok_or_else(|| TelegramError::Protocol {
            method: "sendMediaGroup".to_owned(),
            detail: "response is not a message list".to_owned(),
        })?;
        array
            .iter()
            .map(|message| {
                message["message_id"]
                    .as_i64()
                    .ok_or_else(|| TelegramError::Protocol {
                        method: "sendMediaGroup".to_owned(),
                        detail: "message list has no id".to_owned(),
                    })
            })
            .collect()
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
        let mut url = reqwest::Url::parse(&base).map_err(|error| TelegramError::Protocol {
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
