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

impl MediaError {
    /// Whether the same request may succeed later.
    pub fn retryable(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::Response { status, .. } => *status == 429 || (500..=599).contains(status),
            Self::Io(_) | Self::Protocol(_) | Self::Empty(_) => false,
        }
    }
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

/// Guidance for writing ElevenLabs speech with audio tags.
pub const ELEVENLABS_TAG_GUIDANCE: &str = "Use sparse Eleven v4 audio tags in square brackets, such as [warm, composed voice], [softly], [laughs], or [whispers], where they fit. Never use SSML, and never read the tags aloud.";

/// Turns text into an OGG/Opus voice reply.
pub trait Speaker: Send + Sync {
    /// Return the audio bytes for one text.
    fn speak(&self, text: &str) -> Result<Vec<u8>, MediaError>;
}

/// The ElevenLabs text-to-speech client.
pub struct SpeechClient {
    base_url: String,
    api_key: String,
    voice_id: String,
    model: String,
    output_format: String,
    client: reqwest::blocking::Client,
}

impl SpeechClient {
    /// Build a client. The endpoint is `{base}/v1/text-to-speech/{voice}`.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        voice_id: impl Into<String>,
        model: impl Into<String>,
        output_format: impl Into<String>,
    ) -> Result<Self, MediaError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| MediaError::Transport(format!("client build failed: {error}")))?;
        Ok(Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            voice_id: voice_id.into(),
            model: model.into(),
            output_format: output_format.into(),
            client,
        })
    }

    /// Build a client for the public ElevenLabs API with the default model and format.
    pub fn elevenlabs(
        api_key: impl Into<String>,
        voice_id: impl Into<String>,
    ) -> Result<Self, MediaError> {
        Self::new(
            "https://api.elevenlabs.io",
            api_key,
            voice_id,
            "eleven_v4",
            "opus_48000_64",
        )
    }
}

impl Speaker for SpeechClient {
    fn speak(&self, text: &str) -> Result<Vec<u8>, MediaError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/v1/text-to-speech/",
            self.base_url.trim_end_matches('/')
        ))
        .map_err(|error| MediaError::Protocol(format!("invalid speech base URL: {error}")))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| MediaError::Protocol("speech URL cannot take a path".to_owned()))?;
            segments.pop_if_empty();
            segments.push(&self.voice_id);
        }
        url.query_pairs_mut()
            .append_pair("output_format", &self.output_format);
        let response = self
            .client
            .post(url)
            .header("xi-api-key", &self.api_key)
            .json(&serde_json::json!({
                "text": text,
                "model_id": self.model,
            }))
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
        let bytes = response
            .bytes()
            .map_err(|error| MediaError::Transport(format!("could not read audio: {error}")))?;
        if bytes.is_empty() {
            return Err(MediaError::Empty(
                "the speech response was empty".to_owned(),
            ));
        }
        Ok(bytes.to_vec())
    }
}

/// Generates images through an OpenRouter Images-compatible endpoint.
pub struct ImageClient {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl ImageClient {
    /// Build a client. The endpoint is `{base}/images/generations`.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, MediaError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| MediaError::Transport(format!("client build failed: {error}")))?;
        Ok(Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            client,
        })
    }

    /// Build a client for the public OpenRouter Images API.
    pub fn openrouter(
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, MediaError> {
        Self::new("https://openrouter.ai/api/v1", api_key, model)
    }

    /// Generate one image and return its encoded bytes.
    pub fn generate(&self, prompt: &str, size: Option<&str>) -> Result<Vec<u8>, MediaError> {
        let mut body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
        });
        if let Some(size) = size {
            body["size"] = serde_json::json!(size);
        }
        let url = format!("{}/images/generations", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
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
        let value: Value = response
            .json()
            .map_err(|error| MediaError::Protocol(format!("invalid JSON: {error}")))?;
        let data = value["data"]
            .as_array()
            .and_then(|items| items.first())
            .ok_or_else(|| MediaError::Empty("the image response had no data".to_owned()))?;
        if let Some(encoded) = data["b64_json"].as_str() {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| MediaError::Protocol(format!("invalid image base64: {error}")))?;
            if bytes.is_empty() {
                return Err(MediaError::Empty("the image was empty".to_owned()));
            }
            return Ok(bytes);
        }
        if let Some(url) = data["url"].as_str() {
            let response = self.client.get(url).send().map_err(|error| {
                MediaError::Transport(format!("image download failed: {error}"))
            })?;
            let status = response.status();
            if !status.is_success() {
                return Err(MediaError::Response {
                    status: status.as_u16(),
                    message: "image download was rejected".to_owned(),
                });
            }
            let bytes = response.bytes().map_err(|error| {
                MediaError::Transport(format!("could not read the image: {error}"))
            })?;
            if bytes.is_empty() {
                return Err(MediaError::Empty(
                    "the downloaded image was empty".to_owned(),
                ));
            }
            return Ok(bytes.to_vec());
        }
        Err(MediaError::Protocol(
            "the image response had neither b64_json nor url".to_owned(),
        ))
    }
}
