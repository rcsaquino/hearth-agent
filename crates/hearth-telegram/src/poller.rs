//! Telegram polling and durable admission.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hearth_core::{CoreError, Session, TelegramUpdate};
use hearth_model::Transcriber;
use serde_json::{Value, json};
use thiserror::Error;

use crate::api::{TelegramClient, TelegramError};
use crate::media_store::InboundMedia;
use crate::types::{Message, Update};

/// The largest attachment Telegram may deliver to Hearth.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

/// The most queued submissions one chat may have before Hearth turns it away.
pub const MAX_QUEUED_PER_CHAT: usize = 10;

const BUSY_NOTICE: &str = "I'm busy with the current queue. Try again shortly.";

const ALLOWED_UPDATES: &[&str] = &["message", "callback_query"];

/// How long an album waits for another member before it is admitted.
const ALBUM_QUIET_WINDOW: Duration = Duration::from_millis(500);

/// The longest an album waits for a member that arrives slowly.
const ALBUM_MAX_WAIT: Duration = Duration::from_secs(2);

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

    /// Poll once and admit authorized messages, holding an album until its
    /// members arrive.
    pub fn poll_once(&self) -> Result<PollReport, PollError> {
        let offset = self.session.telegram_offset()?;
        let updates = self.collect_updates(offset)?;
        let mut report = PollReport {
            next_offset: offset,
            ..PollReport::default()
        };
        let mut cursor = offset;
        for item in batch_items(&updates) {
            cursor = cursor.max(item.next_offset);
            report.next_offset = cursor;
            let Some(primary) = item.messages.first().copied() else {
                self.session.set_telegram_offset(cursor)?;
                report.ignored += 1;
                continue;
            };
            if !self.authorized(primary) {
                tracing::warn!(
                    update_id = item.update_id,
                    "ignored unauthorized telegram update"
                );
                self.session.set_telegram_offset(cursor)?;
                report.ignored += 1;
                continue;
            }
            if item.messages.iter().any(|message| {
                message
                    .text
                    .as_deref()
                    .is_some_and(|text| text.trim().eq_ignore_ascii_case("stop"))
            }) {
                let recorded = self.session.record_telegram_update(
                    TelegramUpdate {
                        update_id: item.update_id,
                        chat_id: primary.chat.id,
                        message_id: primary.message_id,
                    },
                    cursor,
                )?;
                if !recorded {
                    self.session.set_telegram_offset(cursor)?;
                    report.duplicates += 1;
                    continue;
                }
                let active = self
                    .stop
                    .as_ref()
                    .is_some_and(|handler| handler.stop(primary.chat.id));
                tracing::info!(chat_id = primary.chat.id, active, "telegram stop");
                report.stopped += 1;
                continue;
            }
            if self.session.telegram_update_seen(item.update_id)? {
                self.session.set_telegram_offset(cursor)?;
                report.duplicates += 1;
                continue;
            }

            if !has_content(&item.messages) {
                self.session.set_telegram_offset(cursor)?;
                report.ignored += 1;
                continue;
            }

            let thread_id = primary.message_thread_id.unwrap_or(0);
            let key = format!("{}:{}", primary.chat.id, thread_id);
            let conversation = match self.session.conversation_by_transport("telegram", &key)? {
                Some(conversation) => conversation,
                None => {
                    let title = primary
                        .chat
                        .title
                        .clone()
                        .or_else(|| {
                            primary
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
                    .map(|mut chats| chats.insert(primary.chat.id))
                    .unwrap_or(false);
                if notify
                    && let Err(error) = self.client.send_message(
                        primary.chat.id,
                        primary.message_thread_id,
                        BUSY_NOTICE,
                        &[],
                    )
                {
                    tracing::warn!(%error, "telegram busy notice failed");
                }
                let recorded = self.session.record_telegram_update(
                    TelegramUpdate {
                        update_id: item.update_id,
                        chat_id: primary.chat.id,
                        message_id: primary.message_id,
                    },
                    cursor,
                )?;
                if !recorded {
                    self.session.set_telegram_offset(cursor)?;
                    report.duplicates += 1;
                } else {
                    report.busy += 1;
                }
                continue;
            }
            if let Ok(mut chats) = self.busy_notified.lock() {
                chats.remove(&primary.chat.id);
            }

            let blocks = self.blocks(&item.messages);
            if blocks.is_empty() {
                self.session.set_telegram_offset(cursor)?;
                report.ignored += 1;
                continue;
            }

            let request_id = format!("tg:{}:{}", primary.chat.id, primary.message_id);
            if let Some(handler) = self.stop.as_ref() {
                handler.admit(primary.chat.id);
            }
            let admission = self.session.submit_telegram(
                &conversation.id,
                &request_id,
                Value::Array(blocks),
                TelegramUpdate {
                    update_id: item.update_id,
                    chat_id: primary.chat.id,
                    message_id: primary.message_id,
                },
                cursor,
            )?;
            if admission.duplicate {
                self.session.set_telegram_offset(cursor)?;
                report.duplicates += 1;
                continue;
            }
            report.admitted += 1;
        }
        Ok(report)
    }

    /// Read the batch, and give an album a moment to deliver every member.
    fn collect_updates(&self, offset: i64) -> Result<Vec<Update>, TelegramError> {
        let mut updates = BTreeMap::new();
        for update in self
            .client
            .get_updates(offset, self.poll_timeout, ALLOWED_UPDATES)?
        {
            updates.insert(update.update_id, update);
        }
        if !updates.values().any(is_album_update) {
            return Ok(updates.into_values().collect());
        }
        let deadline = Instant::now() + ALBUM_MAX_WAIT;
        loop {
            thread::sleep(ALBUM_QUIET_WINDOW);
            let before = updates.len();
            for update in self.client.get_updates(offset, 0, ALLOWED_UPDATES)? {
                updates.insert(update.update_id, update);
            }
            if updates.len() == before || Instant::now() >= deadline {
                break;
            }
        }
        Ok(updates.into_values().collect())
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

        let text = messages
            .iter()
            .filter_map(|message| message_text(message))
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

/// One admission: a single message, or one album's members.
struct BatchItem<'a> {
    messages: Vec<&'a Message>,
    update_id: i64,
    next_offset: i64,
}

/// Collapse one album's updates into one admission item.
fn batch_items(updates: &[Update]) -> Vec<BatchItem<'_>> {
    let mut items: Vec<BatchItem> = Vec::new();
    let mut albums: HashMap<(i64, &str), usize> = HashMap::new();
    for update in updates {
        let next_offset = update.update_id + 1;
        let Some(message) = update.message.as_ref() else {
            items.push(BatchItem {
                messages: Vec::new(),
                update_id: update.update_id,
                next_offset,
            });
            continue;
        };
        let album = message
            .media_group_id
            .as_deref()
            .map(|group| (message.chat.id, group));
        if let Some(index) = album.and_then(|key| albums.get(&key).copied()) {
            let item = &mut items[index];
            item.messages.push(message);
            item.next_offset = item.next_offset.max(next_offset);
            continue;
        }
        if let Some(key) = album {
            albums.insert(key, items.len());
        }
        items.push(BatchItem {
            messages: vec![message],
            update_id: update.update_id,
            next_offset,
        });
    }
    items
}

fn is_album_update(update: &Update) -> bool {
    update
        .message
        .as_ref()
        .is_some_and(|message| message.media_group_id.is_some())
}

fn file_block(path: &std::path::Path, name: &str, mime: &str) -> Value {
    json!({
        "type": "file",
        "path": path.to_string_lossy(),
        "name": name,
        "mime": mime,
    })
}
