//! Hearth Telegram: the Bot API client, polling, and admission.

mod api;
mod format;
mod media_store;
mod message_tool;
mod poller;
mod sender;
mod types;

pub use api::{MediaKind, TelegramClient, TelegramError};
pub use format::{Entity, FormattedText, format_markdown, split_formatted};
pub use media_store::{InboundMedia, MediaStoreError};
pub use message_tool::MessageTool;
pub use poller::{PollError, PollReport, StopHandler, TelegramPoller};
pub use sender::{SendOutcome, Sleeper, SystemSleeper, TelegramSender};
pub use types::{Chat, Message, Update, User};
