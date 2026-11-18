//! Telegram polling and durable admission.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hearth_core::{Conversation, CoreError, Session, TelegramUpdate};
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

const ALLOWED_UPDATES: &[&str] = &["message", "callback_query"];

/// How long an album waits after its last member before it is admitted.
pub const ALBUM_QUIET_MICROS: i64 = 1_000_000;

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
    /// Ask the chat's active run to stop. Returns whether a run was registered.
    fn stop(&self, chat_id: i64) -> bool;

    /// A new message was admitted, so a stop recorded before it does not apply.
    fn admit(&self, chat_id: i64);
}

/// What one poll changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PollReport {
    pub admitted: usize,
    pub staged: usize,
    pub albums_pending: usize,
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
    ///
    /// A due album is submitted before the new batch is read. An album member
    /// is staged for a later poll, so the poller never sleeps on an album.
    pub fn poll_once(&self) -> Result<PollReport, PollError> {
        let offset = self.session.telegram_offset()?;
        let mut report = PollReport {
            next_offset: offset,
            ..PollReport::default()
        };
        report.admitted = self.flush_albums(self.session.now_micros())?;
        let timeout = if self.session.telegram_album_pending()? {
            0
        } else {
            self.poll_timeout
        };
        let updates = self.client.get_updates(offset, timeout, ALLOWED_UPDATES)?;
        for update in updates {
            let next_offset = update.update_id + 1;
            report.next_offset = report.next_offset.max(next_offset);
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

            if !has_content(&[message]) {
                self.session.set_telegram_offset(next_offset)?;
                report.ignored += 1;
                continue;
            }

            if let Some(group_id) = message.media_group_id.as_deref() {
                let ready_at = self.session.now_micros() + ALBUM_QUIET_MICROS;
                let message_json = serde_json::to_value(message)
                    .map_err(|error| PollError::Core(CoreError::Invalid(error.to_string())))?;
                let staged = self.session.stage_telegram_album_member(
                    group_id,
                    message.message_thread_id,
                    TelegramUpdate {
                        update_id: update.update_id,
                        chat_id: message.chat.id,
                        message_id: message.message_id,
                    },
                    message_json,
                    ready_at,
                    next_offset,
                )?;
                if staged {
                    report.staged += 1;
                } else {
                    self.session.set_telegram_offset(next_offset)?;
                    report.duplicates += 1;
                }
                continue;
            }

            let conversation = self.conversation_for(message)?;
            if self.session.queued_submission_count(&conversation.id)? >= MAX_QUEUED_PER_CHAT {
                self.notify_busy(message);
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

            let blocks = self.blocks(&[message]);
            if blocks.is_empty() {
                self.session.set_telegram_offset(next_offset)?;
                report.ignored += 1;
                continue;
            }

            let request_id = format!("tg:{}:{}", message.chat.id, message.message_id);
            if let Some(handler) = self.stop.as_ref() {
                handler.admit(message.chat.id);
            }
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
        }
        report.albums_pending = usize::from(self.session.telegram_album_pending()?);
        Ok(report)
    }

    /// Submit every staged album whose quiet window has passed.
    fn flush_albums(&self, now: i64) -> Result<usize, PollError> {
        let due = self.session.due_telegram_albums(now)?;
        let mut admitted = 0;
        for album in due {
            let messages: Vec<Message> = match album
                .members
                .iter()
                .map(|member| serde_json::from_value(member.message.clone()))
                .collect()
            {
                Ok(messages) => messages,
                Err(error) => {
                    tracing::warn!(%error, "a staged album member could not be read");
                    self.session
                        .clear_telegram_album(album.chat_id, &album.group_id)?;
                    continue;
                }
            };
            let Some(first) = messages.first() else {
                self.session
                    .clear_telegram_album(album.chat_id, &album.group_id)?;
                continue;
            };
            let conversation = self.conversation_for(first)?;
            if self.session.queued_submission_count(&conversation.id)? >= MAX_QUEUED_PER_CHAT {
                self.notify_busy(first);
                continue;
            }
            let refs: Vec<&Message> = messages.iter().collect();
            let blocks = self.blocks(&refs);
            if blocks.is_empty() {
                self.session
                    .clear_telegram_album(album.chat_id, &album.group_id)?;
                continue;
            }
            let request_id = format!("tg:{}:{}", album.chat_id, album.members[0].message_id);
            if self
                .session
                .submission_by_request_id(&request_id)?
                .is_some()
            {
                self.session
                    .clear_telegram_album(album.chat_id, &album.group_id)?;
                continue;
            }
            if let Some(handler) = self.stop.as_ref() {
                handler.admit(album.chat_id);
            }
            let admission = self.session.submit_telegram_album(
                &conversation.id,
                &request_id,
                Value::Array(blocks),
                &album.members[0].message_id.to_string(),
                album.chat_id,
                &album.group_id,
            )?;
            if admission.duplicate {
                self.session
                    .clear_telegram_album(album.chat_id, &album.group_id)?;
            } else {
                admitted += 1;
            }
            if let Ok(mut chats) = self.busy_notified.lock() {
                chats.remove(&album.chat_id);
            }
        }
        Ok(admitted)
    }

    fn conversation_for(&self, message: &Message) -> Result<Conversation, CoreError> {
        let thread_id = message.message_thread_id.unwrap_or(0);
        let key = format!("{}:{}", message.chat.id, thread_id);
        match self.session.conversation_by_transport("telegram", &key)? {
            Some(conversation) => Ok(conversation),
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
                self.session.create_conversation(&title, "telegram", &key)
            }
        }
    }

    fn notify_busy(&self, message: &Message) {
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
    }

    fn blocks(&self, messages: &[&Message]) -> Vec<Value> {
        let mut blocks = Vec::new();
        if let Some(context) = messages.iter().find_map(|message| {
            message
                .reply_to_message
                .as_deref()
                .and_then(reply_context)
                .or_else(|| message.external_reply.as_ref().and_then(quoted_span))
        }) {
            blocks.push(json!({"type": "text", "text": context}));
        }

        let mut seen = HashSet::new();
        let text = messages
            .iter()
            .filter_map(|message| message_text(message))
            .filter(|text| seen.insert(text.clone()))
            .collect::<Vec<_>>()
            .join("\n\n");
        if !text.is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }

        for message in messages {
            self.media_blocks(message, &mut blocks);
        }
        blocks
    }

    /// Save one message's attachment and append its content block.
    fn media_blocks(&self, message: &Message, blocks: &mut Vec<Value>) {
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

/// The largest quoted body Hearth forwards as reply context.
const REPLY_CONTEXT_CHARS: usize = 500;

/// The text the person typed, with a rich message flattened to plain text.
fn message_text(message: &Message) -> Option<String> {
    if let Some(text) = message
        .text
        .as_deref()
        .or(message.caption.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_owned());
    }
    message.rich_message.as_ref().and_then(rich_text)
}

fn rich_text(rich: &crate::types::RichMessage) -> Option<String> {
    let mut text = String::new();
    for block in &rich.blocks {
        collect_rich_text(block, &mut text);
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
    }
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Collect the text of one rich block, ignoring structural and media fields.
fn collect_rich_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Array(items) => {
            for item in items {
                collect_rich_text(item, out);
            }
        }
        Value::Object(fields) => {
            if let Some(text) = fields.get("text").or_else(|| fields.get("caption")) {
                collect_rich_text(text, out);
            } else if let Some(blocks) = fields.get("blocks") {
                collect_rich_text(blocks, out);
            } else if let Some(items) = fields.get("items") {
                collect_rich_text(items, out);
            } else if let Some(cells) = fields.get("cells") {
                collect_rich_text(cells, out);
            }
        }
        _ => {}
    }
}

/// The quoted body, shortened to one context line.
fn quote_body(text: &str) -> String {
    let mut characters = text.chars();
    let head: String = characters.by_ref().take(REPLY_CONTEXT_CHARS).collect();
    if characters.next().is_some() {
        format!("\"{head}…\"")
    } else {
        format!("\"{head}\"")
    }
}

/// The context line describing the message being replied to.
fn reply_context(quoted: &Message) -> Option<String> {
    let author = quoted
        .from
        .as_ref()
        .and_then(|user| user.first_name.as_deref())
        .unwrap_or("someone");
    if let Some(text) = message_text(quoted) {
        return Some(format!("Replying to {author}: {}", quote_body(&text)));
    }
    let body = if !quoted.photo.is_empty() {
        "(a photo)"
    } else if quoted.document.is_some() {
        "(a document)"
    } else if quoted.audio.is_some() {
        "(an audio file)"
    } else if quoted.voice.is_some() {
        "(a voice message)"
    } else if quoted.video.is_some() {
        "(a video)"
    } else if quoted.rich_message.is_some() {
        "(a rich message)"
    } else {
        return None;
    };
    Some(format!("Replying to {author}: {body}"))
}

fn quoted_span(reply: &crate::types::ExternalReply) -> Option<String> {
    let text = reply
        .quote
        .as_ref()
        .and_then(|quote| quote.text.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    Some(format!("Replying to a quoted span: {}", quote_body(text)))
}

fn has_content(messages: &[&Message]) -> bool {
    messages.iter().any(|message| {
        message_text(message).is_some()
            || !message.photo.is_empty()
            || message.document.is_some()
            || message.audio.is_some()
            || message.voice.is_some()
            || message.video.is_some()
    })
}

fn file_block(path: &std::path::Path, name: &str, mime: &str) -> Value {
    json!({
        "type": "file",
        "path": path.to_string_lossy(),
        "name": name,
        "mime": mime,
    })
}
