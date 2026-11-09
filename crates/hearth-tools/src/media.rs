//! The media tool: image generation, transcription, and speech.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hearth_model::{ImageClient, MediaError, Speaker, ToolSpec, Transcriber};
use serde_json::{Value, json};

use crate::tool::{Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// Optional image and speech capabilities built from the media settings.
pub struct MediaTool {
    image: Option<ImageClient>,
    transcriber: Option<Arc<dyn Transcriber>>,
    speaker: Option<Arc<dyn Speaker>>,
    workspace: Workspace,
    outbound: PathBuf,
}

impl MediaTool {
    /// Build the tool. Any missing service is reported only when its action is called.
    pub fn new(
        image: Option<ImageClient>,
        transcriber: Option<Arc<dyn Transcriber>>,
        speaker: Option<Arc<dyn Speaker>>,
        workspace: impl Into<PathBuf>,
        spill: impl Into<PathBuf>,
        media: impl Into<PathBuf>,
        outbound: impl Into<PathBuf>,
    ) -> Self {
        Self {
            image,
            transcriber,
            speaker,
            workspace: Workspace::with_media(workspace, spill, media),
            outbound: outbound.into(),
        }
    }

    fn write_outbound(
        &self,
        prefix: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, ToolError> {
        fs::create_dir_all(&self.outbound)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self.outbound.join(format!("{prefix}-{stamp}.{extension}"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(bytes)?;
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(path)
    }
}

impl Tool for MediaTool {
    fn spec(&self) -> ToolSpec {
        let mut description = "Generate an image, transcribe an audio file, or synthesize a voice reply using the configured media services."
            .to_owned();
        let mut text = "Text for the speak action".to_owned();
        if self.speaker.is_some() {
            description.push(' ');
            description.push_str(hearth_model::ELEVENLABS_TAG_GUIDANCE);
            text.push_str(". ");
            text.push_str(hearth_model::ELEVENLABS_TAG_GUIDANCE);
        }
        ToolSpec {
            name: "media".to_owned(),
            description,
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["image", "transcribe", "speak"]},
                    "prompt": {"type": "string"},
                    "size": {"type": "string"},
                    "path": {"type": "string"},
                    "text": {"type": "string", "description": text}
                },
                "required": ["action"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "action is required"))?;
        match action {
            "image" => {
                let client = self.image.as_ref().ok_or_else(|| {
                    ToolError::invalid("media_not_configured", "image generation is not configured")
                })?;
                let prompt = arguments
                    .get("prompt")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "prompt is required for image")
                    })?;
                let size = arguments.get("size").and_then(Value::as_str);
                let bytes = client.generate(prompt, size).map_err(map_media_error)?;
                let extension = image_extension(&bytes);
                let path = self.write_outbound("image", extension, &bytes)?;
                Ok(ToolOutput {
                    content: format!("image: {}", path.display()),
                    is_error: false,
                })
            }
            "transcribe" => {
                let transcriber = self.transcriber.as_ref().ok_or_else(|| {
                    ToolError::invalid("media_not_configured", "transcription is not configured")
                })?;
                let path = arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "path is required for transcribe")
                    })?;
                let target = self.workspace.resolve_read_path(path)?;
                let text = transcriber.transcribe(&target).map_err(map_media_error)?;
                Ok(ToolOutput {
                    content: text,
                    is_error: false,
                })
            }
            "speak" => {
                let speaker = self.speaker.as_ref().ok_or_else(|| {
                    ToolError::invalid("media_not_configured", "speech is not configured")
                })?;
                let text = arguments
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "text is required for speak")
                    })?;
                let bytes = speaker.speak(text).map_err(map_media_error)?;
                let path = self.write_outbound("voice", "ogg", &bytes)?;
                Ok(ToolOutput {
                    content: format!("voice: {}", path.display()),
                    is_error: false,
                })
            }
            other => Err(ToolError::invalid(
                "unknown_action",
                format!("unknown media action: {other}"),
            )),
        }
    }
}

fn map_media_error(error: MediaError) -> ToolError {
    if error.retryable() {
        ToolError::temporary("media_error", error.to_string())
    } else {
        ToolError::invalid("media_error", error.to_string())
    }
}

fn image_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        "png"
    }
}
