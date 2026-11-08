//! Telegram polling and durable admission.

use std::collections::HashSet;

use hearth_core::{CoreError, Session, TelegramUpdate};
use serde_json::json;
use thiserror::Error;

use crate::api::{TelegramClient, TelegramError};
use crate::types::{Message, Update};

/// A polling failure.
#[derive(Debug, Error)]
pub enum PollError {
    #[error(transparent)]
    Client(#[from] TelegramError),
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// What one poll changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PollReport {
    pub admitted: usize,
    pub duplicates: usize,
    pub ignored: usize,
    pub next_offset: i64,
}

/// The Telegram poller for one Hearth home.
pub struct TelegramPoller {
    client: TelegramClient,
    session: Session,
    allowed_users: HashSet<i64>,
    allowed_chats: HashSet<i64>,
    poll_timeout: u32,
}

impl TelegramPoller {
    /// Build a poller.
    pub fn new(
        client: TelegramClient,
        session: Session,
        allowed_user_ids: impl IntoIterator<Item = i64>,
        allowed_chat_ids: impl IntoIterator<Item = i64>,
        poll_timeout: u32,
    ) -> Self {
        Self {
            client,
            session,
            allowed_users: allowed_user_ids.into_iter().collect(),
            allowed_chats: allowed_chat_ids.into_iter().collect(),
            poll_timeout,
        }
    }

    /// Poll once and admit authorized text messages.
    pub fn poll_once(&self) -> Result<PollReport, PollError> {
        let offset = self.session.telegram_offset()?;
        let updates = self.client.get_updates(
            offset,
            self.poll_timeout,
            &["message", "callback_query"],
        )?;
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
            let Some(text) = message
                .text
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                self.session.set_telegram_offset(next_offset)?;
                report.ignored += 1;
                continue;
            };

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
                    self.session
                        .create_conversation(&title, "telegram", &key)?
                }
            };

            let request_id = format!("tg:{}:{}", message.chat.id, message.message_id);
            let admission = self.session.submit_telegram(
                &conversation.id,
                &request_id,
                json!({"text": text}),
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
            if let Err(error) = self.client.set_message_reaction(
                message.chat.id,
                message.message_id,
                "👀",
            ) {
                tracing::warn!(%error, "telegram reaction failed");
            }
        }
        Ok(report)
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
