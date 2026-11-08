//! Telegram polling and durable admission.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hearth_core::{CoreError, Session, TelegramUpdate};
use hearth_model::Transcriber;
use serde_json::{Value, json};
use thiserror::Error;

use crate::api::{TelegramClient, TelegramError};
use crate::media_store::InboundMedia;
use crate::types::Message;

/// The largest attachment Telegram may deliver to Hearth.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

/// The most queued submissions one chat may have before Hearth turns it away.
pub const MAX_QUEUED_PER_CHAT: usize = 10;

const BUSY_NOTICE: &str = "I'm busy with the current queue. Try again shortly.";

/// A polling failure.
#[derive(Debug, Error)]
pub enum PollError {
    #[error(transparent)]
    Client(#[from] TelegramError),
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// A handler asked to stop the active run in a chat.
pub trait StopHandler: Send + Sync {
    fn stop(&self, chat_id: i64) -> bool;
}

/// What one poll changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PollReport {
    pub admitted: usize,
    pub duplicates: usize,
    pub ignored: usize,
    pub stopped: usize,
    pub busy: usize,
    pub next_offset: i64,
}

/// The Telegram poller for one Hearth home.
pub struct TelegramPoller {
    client: TelegramClient,
    session: Session,
    allowed_users: HashSet<i64>,
    allowed_chats: HashSet<i64>,
    poll_timeout: u32,
    media: InboundMedia,
    transcriber: Option<Arc<dyn Transcriber>>,
    stop: Option<Arc<dyn StopHandler>>,
    busy_notified: Mutex<HashSet<i64>>,
}

impl TelegramPoller {
    /// Build a poller.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: TelegramClient,
        session: Session,
        allowed_user_ids: impl IntoIterator<Item = i64>,
        allowed_chat_ids: impl IntoIterator<Item = i64>,
        poll_timeout: u32,
        media: InboundMedia,
        transcriber: Option<Arc<dyn Transcriber>>,
        stop: Option<Arc<dyn StopHandler>>,
    ) -> Self {
        Self {
            client,
            session,
            allowed_users: allowed_user_ids.into_iter().collect(),
            allowed_chats: allowed_chat_ids.into_iter().collect(),
            poll_timeout,
            media,
            transcriber,
            stop,
            busy_notified: Mutex::new(HashSet::new()),
        }
    }

    /// The session this poller writes through.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Poll once and admit authorized messages.
    pub fn poll_once(&self) -> Result<PollReport, PollError> {
        let offset = self.session.telegram_offset()?;
        let updates =
            self.client
                .get_updates(offset, self.poll_timeout, &["message", "callback_query"])?;
        let mut report = PollReport {
            next_offset: offset,
            ..PollReport::default()
        };
        for update in updates {
            let next_offset = update.update_id + 1;
            report.next_offset = next_offset;
            let Some(message) = update.message.as_ref() else {
                self.session.set_telegram_offset(next_offset)?;
                report.ignored += 1;
                continue;
            };
            if !self.authorized(message) {
                tracing::warn!(
                    update_id = update.update_id,
                    "ignored unauthorized telegram update"
                );
                self.session.set_telegram_offset(next_offset)?;
                report.ignored += 1;
                continue;
            }
            if message
                .text
                .as_deref()
                .is_some_and(|text| text.trim().eq_ignore_ascii_case("stop"))
            {
                let recorded = self.session.record_telegram_update(
                    TelegramUpdate {
                        update_id: update.update_id,
                        chat_id: message.chat.id,
                        message_id: message.message_id,
                    },
                    next_offset,
                )?;
                if !recorded {
                    self.session.set_telegram_offset(next_offset)?;
                    report.duplicates += 1;
                    continue;
                }
                let active = self
                    .stop
                    .as_ref()
                    .is_some_and(|handler| handler.stop(message.chat.id));
                tracing::info!(chat_id = message.chat.id, active, "telegram stop");
                report.stopped += 1;
                continue;
            }
            if self.session.telegram_update_seen(update.update_id)? {
                self.session.set_telegram_offset(next_offset)?;
                report.duplicates += 1;
                continue;
            }

            if !has_content(message) {
                self.session.set_telegram_offset(next_offset)?;
                report.ignored += 1;
                continue;
            }

            let thread_id = message.message_thread_id.unwrap_or(0);
            let key = format!("{}:{}", message.chat.id, thread_id);
            let conversation = match self.session.conversation_by_transport("telegram", &key)? {
                Some(conversation) => conversation,
                None => {
                    let title = message
                        .chat
                        .title
                        .clone()
                        .or_else(|| {
                            message
                                .from
                                .as_ref()
                                .and_then(|user| user.first_name.clone())
                        })
                        .unwrap_or_else(|| "Telegram".to_owned());
                    self.session.create_conversation(&title, "telegram", &key)?
                }
            };

            if self.session.queued_submission_count(&conversation.id)? >= MAX_QUEUED_PER_CHAT {
                let notify = self
                    .busy_notified
                    .lock()
                    .map(|mut chats| chats.insert(message.chat.id))
                    .unwrap_or(false);
                if notify
                    && let Err(error) = self.client.send_message(
                        message.chat.id,
                        message.message_thread_id,
                        BUSY_NOTICE,
                        &[],
                    )
                {
                    tracing::warn!(%error, "telegram busy notice failed");
                }
                let recorded = self.session.record_telegram_update(
                    TelegramUpdate {
                        update_id: update.update_id,
                        chat_id: message.chat.id,
                        message_id: message.message_id,
                    },
                    next_offset,
                )?;
                if !recorded {
                    self.session.set_telegram_offset(next_offset)?;
                    report.duplicates += 1;
                } else {
                    report.busy += 1;
                }
                continue;
            }
            if let Ok(mut chats) = self.busy_notified.lock() {
                chats.remove(&message.chat.id);
            }

            let blocks = self.blocks(message);
            if blocks.is_empty() {
                self.session.set_telegram_offset(next_offset)?;
                report.ignored += 1;
                continue;
            }

            let request_id = format!("tg:{}:{}", message.chat.id, message.message_id);
            let admission = self.session.submit_telegram(
                &conversation.id,
                &request_id,
                Value::Array(blocks),
                TelegramUpdate {
                    update_id: update.update_id,
                    chat_id: message.chat.id,
                    message_id: message.message_id,
                },
                next_offset,
            )?;
            if admission.duplicate {
                self.session.set_telegram_offset(next_offset)?;
                report.duplicates += 1;
                continue;
            }
            report.admitted += 1;
            if let Err(error) =
                self.client
                    .set_message_reaction(message.chat.id, message.message_id, "👀")
            {
                tracing::warn!(%error, "telegram reaction failed");
            }
        }
        Ok(report)
    }

    fn blocks(&self, message: &Message) -> Vec<Value> {
        let mut blocks = Vec::new();
        if let Some(text) = message
            .text
            .as_deref()
            .or(message.caption.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            blocks.push(json!({"type": "text", "text": text}));
        }

        if let Some(photo) = message
            .photo
            .iter()
            .max_by_key(|photo| photo.width.saturating_mul(photo.height))
        {
            match self.save_attachment(message, &photo.file_id, photo.file_size, "photo.jpg") {
                Ok(path) => blocks.push(json!({
                    "type": "image",
                    "path": path.to_string_lossy(),
                    "mime": "image/jpeg",
                })),
                Err(note) => blocks.push(json!({"type": "text", "text": note})),
            }
        }

        if let Some(document) = message.document.as_ref() {
            let name = document
                .file_name
                .clone()
                .unwrap_or_else(|| "document.bin".to_owned());
            let mime = document
                .mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            match self.save_attachment(message, &document.file_id, document.file_size, &name) {
                Ok(path) => blocks.push(file_block(&path, &name, &mime)),
                Err(note) => blocks.push(json!({"type": "text", "text": note})),
            }
        }

        if let Some(audio) = message.audio.as_ref() {
            let name = audio
                .file_name
                .clone()
                .unwrap_or_else(|| "audio.bin".to_owned());
            let mime = audio
                .mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            match self.save_attachment(message, &audio.file_id, audio.file_size, &name) {
                Ok(path) => blocks.push(file_block(&path, &name, &mime)),
                Err(note) => blocks.push(json!({"type": "text", "text": note})),
            }
        }

        if let Some(voice) = message.voice.as_ref() {
            let mime = voice
                .mime_type
                .clone()
                .unwrap_or_else(|| "audio/ogg".to_owned());
            match self.save_attachment(message, &voice.file_id, voice.file_size, "voice.ogg") {
                Ok(path) => {
                    match self.transcriber.as_ref() {
                        Some(transcriber) => match transcriber.transcribe(&path) {
                            Ok(text) if !text.trim().is_empty() => {
                                blocks.push(json!({"type": "text", "text": text.trim()}));
                            }
                            Ok(_) => blocks.push(json!({
                                "type": "text",
                                "text": "Voice message transcription was empty."
                            })),
                            Err(error) => blocks.push(json!({
                                "type": "text",
                                "text": format!("Voice message could not be transcribed: {error}")
                            })),
                        },
                        None => blocks.push(json!({
                            "type": "text",
                            "text": "Voice message was not transcribed."
                        })),
                    }
                    let saved = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("voice.ogg");
                    blocks.push(file_block(&path, saved, &mime));
                }
                Err(note) => blocks.push(json!({"type": "text", "text": note})),
            }
        }

        if let Some(video) = message.video.as_ref() {
            let name = video
                .file_name
                .clone()
                .unwrap_or_else(|| "video.bin".to_owned());
            let mime = video
                .mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            match self.save_attachment(message, &video.file_id, video.file_size, &name) {
                Ok(path) => blocks.push(file_block(&path, &name, &mime)),
                Err(note) => blocks.push(json!({"type": "text", "text": note})),
            }
        }

        blocks
    }

    fn save_attachment(
        &self,
        message: &Message,
        file_id: &str,
        file_size: Option<i64>,
        name: &str,
    ) -> Result<PathBuf, String> {
        if file_size.is_some_and(|size| size > MAX_ATTACHMENT_BYTES as i64) {
            return Err(format!(
                "Attachment too large to download: {name} ({} bytes).",
                file_size.unwrap_or_default()
            ));
        }
        let info = self
            .client
            .get_file(file_id)
            .map_err(|error| format!("Could not get file {name}: {error}"))?;
        if info
            .file_size
            .is_some_and(|size| size > MAX_ATTACHMENT_BYTES as i64)
        {
            return Err(format!(
                "Attachment too large to download: {name} ({} bytes).",
                info.file_size.unwrap_or_default()
            ));
        }
        let file_path = info
            .file_path
            .ok_or_else(|| format!("Could not download {name}: no file path."))?;
        let bytes = self
            .client
            .download_file(&file_path, MAX_ATTACHMENT_BYTES)
            .map_err(|error| format!("Could not download {name}: {error}"))?;
        self.media
            .save(message.date, message.message_id, name, &bytes)
            .map_err(|error| format!("Could not save {name}: {error}"))
    }

    fn authorized(&self, message: &Message) -> bool {
        let Some(from) = message.from.as_ref() else {
            return false;
        };
        if from.is_bot == Some(true) {
            return false;
        }
        if !self.allowed_users.contains(&from.id) {
            return false;
        }
        if !self.allowed_chats.is_empty() && !self.allowed_chats.contains(&message.chat.id) {
            return false;
        }
        true
    }
}

fn has_content(message: &Message) -> bool {
    message
        .text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        || message
            .caption
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        || !message.photo.is_empty()
        || message.document.is_some()
        || message.audio.is_some()
        || message.voice.is_some()
        || message.video.is_some()
}

fn file_block(path: &std::path::Path, name: &str, mime: &str) -> Value {
    json!({
        "type": "file",
        "path": path.to_string_lossy(),
        "name": name,
        "mime": mime,
    })
}
