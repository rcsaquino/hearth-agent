//! Hearth Telegram: the Bot API client, polling, and admission.

mod api;
mod format;
mod poller;
mod types;

pub use api::{TelegramClient, TelegramError};
pub use format::{Entity, FormattedText, format_markdown, split_formatted};
pub use poller::{PollError, PollReport, TelegramPoller};
pub use types::{Chat, Message, Update, User};
