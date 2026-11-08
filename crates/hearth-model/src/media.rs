//! Speech and transcription service clients.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use reqwest::blocking::multipart::{Form, Part};
use serde_json::Value;
use thiserror::Error;

/// A media service failure.
#[derive(Debug, Error)]
pub enum MediaError {
    #[error("media file could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("media request failed: {0}")]
    Transport(String),
    #[error("media service returned {status}: {message}")]
    Response { status: u16, message: String },
    #[error("media response was invalid: {0}")]
    Protocol(String),
    #[error("media response was empty: {0}")]
    Empty(String),
}

/// Turns a voice file into text.
pub trait Transcriber: Send + Sync {
    /// Return the transcript for one audio file.
    fn transcribe(&self, path: &Path) -> Result<String, MediaError>;
}

/// The Groq speech-to-text client.
pub struct TranscriptionClient {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl TranscriptionClient {
    /// Build a client. The endpoint is `{base}/openai/v1/audio/transcriptions`.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, MediaError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| MediaError::Transport(format!("client build failed: {error}")))?;
        Ok(Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            client,
        })
    }

    /// Build a client for the public Groq API.
    pub fn groq(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self, MediaError> {
        Self::new("https://api.groq.com", api_key, model)
    }
}

impl Transcriber for TranscriptionClient {
    fn transcribe(&self, path: &Path) -> Result<String, MediaError> {
        let bytes = std::fs::read(path)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("voice.ogg");
        let form = Form::new()
            .part("file", Part::bytes(bytes).file_name(name.to_owned()))
            .text("model", self.model.clone());
        let url = format!(
            "{}/openai/v1/audio/transcriptions",
            self.base_url.trim_end_matches('/')
        );
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .map_err(|error| MediaError::Transport(format!("request failed: {error}")))?;
        let status = response.status();
        if !status.is_success() {
            let mut body = String::new();
            let _ = response.take(400).read_to_string(&mut body);
            return Err(MediaError::Response {
                status: status.as_u16(),
                message: body.chars().take(200).collect(),
            });
        }
        let body: Value = response
            .json()
            .map_err(|error| MediaError::Protocol(format!("invalid JSON: {error}")))?;
        let text = body["text"].as_str().unwrap_or_default().trim();
        if text.is_empty() {
            return Err(MediaError::Empty("the transcript was empty".to_owned()));
        }
        Ok(text.to_owned())
    }
}
