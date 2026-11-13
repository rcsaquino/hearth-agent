//! The message tool: send files, albums, and voice replies.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hearth_model::{MediaError, Speaker, ToolSpec};
use hearth_tools::{Tool, ToolError, ToolOutput, Workspace};
use serde_json::{Value, json};

use crate::api::{MediaKind, TelegramError};
use crate::sender::TelegramSender;

/// Sends files and voice replies to one Telegram chat.
pub struct MessageTool {
    sender: TelegramSender,
    chat_id: i64,
    thread_id: Option<i64>,
    workspace: Workspace,
    outbound: PathBuf,
    speaker: Option<Arc<dyn Speaker>>,
}

impl MessageTool {
    /// Build a message tool for one chat.
    pub fn new(
        sender: TelegramSender,
        chat_id: i64,
        thread_id: Option<i64>,
        workspace: impl Into<PathBuf>,
        spill: impl Into<PathBuf>,
        outbound: impl Into<PathBuf>,
        speaker: Option<Arc<dyn Speaker>>,
    ) -> Self {
        let outbound = outbound.into();
        let media_root = outbound
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| outbound.clone());
        Self {
            sender,
            chat_id,
            thread_id,
            workspace: Workspace::with_media(workspace, spill, media_root),
            outbound,
            speaker,
        }
    }
}

impl Tool for MessageTool {
    fn spec(&self) -> ToolSpec {
        let mut description = "Send files, an album, or a voice reply to the user. Use voice only when the user explicitly asks for it."
            .to_owned();
        let speech = "Text to synthesize as a voice reply";
        if self.speaker.is_some() {
            description.push(' ');
            description.push_str(hearth_model::ELEVENLABS_TAG_GUIDANCE);
        }
        ToolSpec {
            name: "message".to_owned(),
            description,
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "One file inside the workspace or a generated or inbound media file"},
                    "paths": {
                        "type": "array",
                        "items": {"type": "string"},
                        "maxItems": 10,
                        "description": "Two to ten compatible files from the workspace or media"
                    },
                    "caption": {"type": "string", "description": "Only when the user asked for a caption"},
                    "speech": {"type": "string", "description": speech}
                }
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let object = arguments.as_object().ok_or_else(|| {
            ToolError::invalid("invalid_arguments", "arguments must be an object")
        })?;
        let path = object.get("path").and_then(Value::as_str);
        let paths = object.get("paths").and_then(Value::as_array);
        let caption = object
            .get("caption")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let speech = object
            .get("speech")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());

        if path.is_some() && paths.is_some() {
            return Err(ToolError::invalid(
                "invalid_arguments",
                "use path or paths, not both",
            ));
        }
        if path.is_none() && paths.is_none() && speech.is_none() {
            return Err(ToolError::invalid(
                "invalid_arguments",
                "provide a path, paths, or speech",
            ));
        }

        let mut sent = Vec::new();
        if let Some(speech) = speech {
            let speaker = self.speaker.as_ref().ok_or_else(|| {
                ToolError::invalid("speech_not_configured", "speech is not configured")
            })?;
            let audio = speaker.speak(speech).map_err(media_error)?;
            let voice = self.write_voice(&audio)?;
            let id = self
                .sender
                .send_voice(self.chat_id, self.thread_id, &voice, caption)
                .map_err(telegram_error)?;
            sent.push(id);
        }

        if let Some(single) = path {
            let (kind, resolved) = self.resolve(single)?;
            let id = self.send_one(kind, &resolved, caption)?;
            sent.push(id);
        } else if let Some(multiple) = paths {
            if multiple.is_empty() || multiple.len() > 10 {
                return Err(ToolError::invalid(
                    "invalid_arguments",
                    "paths must contain one to ten files",
                ));
            }
            let mut resolved = Vec::with_capacity(multiple.len());
            for item in multiple {
                let value = item.as_str().ok_or_else(|| {
                    ToolError::invalid("invalid_arguments", "each path must be a string")
                })?;
                resolved.push(self.resolve(value)?);
            }
            if resolved.len() == 1 {
                let (kind, path) = &resolved[0];
                sent.push(self.send_one(*kind, path, caption)?);
            } else if resolved.iter().all(|(kind, _)| kind.album_type().is_some()) {
                let items: Vec<(MediaKind, PathBuf)> = resolved;
                let ids = self
                    .sender
                    .send_media_group(self.chat_id, self.thread_id, &items, caption)
                    .map_err(telegram_error)?;
                sent.extend(ids);
            } else {
                for (index, (kind, path)) in resolved.iter().enumerate() {
                    let item_caption = if index == 0 { caption } else { None };
                    sent.push(self.send_one(*kind, path, item_caption)?);
                }
            }
        }

        Ok(ToolOutput {
            content: json!({"sent": sent}).to_string(),
            is_error: false,
        })
    }
}

impl MessageTool {
    fn resolve(&self, path: &str) -> Result<(MediaKind, PathBuf), ToolError> {
        let resolved = self.workspace.resolve_read_path(path)?;
        if !resolved.is_file() {
            return Err(ToolError::invalid(
                "not_a_file",
                format!("not a file: {path}"),
            ));
        }
        Ok((kind_for(&resolved), resolved))
    }

    fn send_one(
        &self,
        kind: MediaKind,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, ToolError> {
        let result = match kind {
            MediaKind::Photo => self
                .sender
                .send_photo(self.chat_id, self.thread_id, path, caption),
            MediaKind::Video => self
                .sender
                .send_video(self.chat_id, self.thread_id, path, caption),
            MediaKind::Voice => self
                .sender
                .send_voice(self.chat_id, self.thread_id, path, caption),
            MediaKind::Document => {
                self.sender
                    .send_document(self.chat_id, self.thread_id, path, caption)
            }
        };
        result.map_err(telegram_error)
    }

    fn write_voice(&self, audio: &[u8]) -> Result<PathBuf, ToolError> {
        fs::create_dir_all(&self.outbound)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self.outbound.join(format!("voice-{nanos}.ogg"));
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(audio)?;
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(path)
    }
}

fn kind_for(path: &Path) -> MediaKind {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "webp" => MediaKind::Photo,
        "mp4" | "mov" | "m4v" => MediaKind::Video,
        "ogg" | "opus" => MediaKind::Voice,
        _ => MediaKind::Document,
    }
}

fn telegram_error(error: TelegramError) -> ToolError {
    let retryable = match &error {
        TelegramError::Transport(_) => true,
        TelegramError::Api { code, .. } => {
            matches!(code, Some(429) | Some(500..=599))
        }
        TelegramError::Protocol { .. } | TelegramError::TooLarge { .. } => false,
    };
    if retryable {
        ToolError::temporary("telegram", error.to_string())
    } else {
        ToolError::invalid("telegram", error.to_string())
    }
}

fn media_error(error: MediaError) -> ToolError {
    if error.retryable() {
        ToolError::temporary("speech", error.to_string())
    } else {
        ToolError::invalid("speech", error.to_string())
    }
}
