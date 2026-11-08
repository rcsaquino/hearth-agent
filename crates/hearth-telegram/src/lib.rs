//! Hearth Telegram: the Bot API client, polling, and admission.

mod api;
mod poller;
mod types;

pub use api::{TelegramClient, TelegramError};
pub use poller::{PollError, PollReport, TelegramPoller};
pub use types::{Chat, Message, Update, User};
