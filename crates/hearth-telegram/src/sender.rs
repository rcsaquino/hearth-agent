//! Telegram reply delivery with a rich-first path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::{MediaKind, TelegramClient, TelegramError};
use crate::format::{format_markdown, split_formatted};

const RICH_TEXT_LIMIT: usize = 32_768;
const RICH_BLOCK_LIMIT: usize = 500;
const SEND_INTERVAL: Duration = Duration::from_millis(33);

/// A blocking delay, injected so tests do not wait.
pub trait Sleeper: Send + Sync {
    fn sleep(&self, duration: Duration);
}

/// The production sleeper.
pub struct SystemSleeper;

impl Sleeper for SystemSleeper {
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// What happened to one reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Rich,
    Fallback { parts: usize },
    Uncertain { message: String },
}

/// Sends final answers, rich first.
#[derive(Clone)]
pub struct TelegramSender {
    client: TelegramClient,
    limiter: RateLimiter,
    sleeper: Arc<dyn Sleeper>,
}

impl TelegramSender {
    /// Build a sender with an injected sleeper.
    pub fn new(client: TelegramClient, sleeper: Arc<dyn Sleeper>) -> Self {
        Self {
            client,
            limiter: RateLimiter::new(),
            sleeper,
        }
    }

    /// Deliver one final answer.
    pub fn send_reply(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        markdown: &str,
    ) -> Result<SendOutcome, TelegramError> {
        if rich_fits(markdown) {
            let mut attempts = 0;
            loop {
                self.limiter.wait(&*self.sleeper);
                match self.client.send_rich_message(chat_id, thread_id, markdown) {
                    Ok(()) => return Ok(SendOutcome::Rich),
                    Err(
                        error @ TelegramError::Api {
                            code: Some(429),
                            retry_after,
                            ..
                        },
                    ) => {
                        if attempts >= 2 {
                            return Err(error);
                        }
                        self.sleeper
                            .sleep(Duration::from_secs(retry_after.unwrap_or(1)));
                        attempts += 1;
                    }
                    Err(TelegramError::Api {
                        code: Some(code), ..
                    }) if (400..500).contains(&code) => {
                        return self.send_fallback(chat_id, thread_id, markdown);
                    }
                    Err(TelegramError::Api {
                        code: Some(500..=599),
                        ..
                    }) => {
                        return Ok(SendOutcome::Uncertain {
                            message: "the Telegram server failed after the request".to_owned(),
                        });
                    }
                    Err(TelegramError::Transport(message)) => {
                        return Ok(SendOutcome::Uncertain {
                            message: format!("delivery is uncertain: {message}"),
                        });
                    }
                    Err(TelegramError::Protocol { .. }) => {
                        return Ok(SendOutcome::Uncertain {
                            message: "delivery is uncertain: invalid Telegram response".to_owned(),
                        });
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        self.send_fallback(chat_id, thread_id, markdown)
    }

    /// Show the typing indicator.
    pub fn send_typing(&self, chat_id: i64, thread_id: Option<i64>) -> Result<(), TelegramError> {
        self.limiter.wait(&*self.sleeper);
        self.client.send_chat_action(chat_id, thread_id, "typing")
    }

    /// Send one photo.
    pub fn send_photo(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.client.send_photo(chat_id, thread_id, path, caption)
    }

    /// Send one document.
    pub fn send_document(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.client.send_document(chat_id, thread_id, path, caption)
    }

    /// Send one video.
    pub fn send_video(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.client.send_video(chat_id, thread_id, path, caption)
    }

    /// Send one voice file.
    pub fn send_voice(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        self.client.send_voice(chat_id, thread_id, path, caption)
    }

    /// Send one album.
    pub fn send_media_group(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        items: &[(MediaKind, PathBuf)],
        caption: Option<&str>,
    ) -> Result<Vec<i64>, TelegramError> {
        self.client
            .send_media_group(chat_id, thread_id, items, caption)
    }

    fn send_fallback(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        markdown: &str,
    ) -> Result<SendOutcome, TelegramError> {
        let formatted = format_markdown(markdown);
        let parts = split_formatted(&formatted, 4096);
        let mut sent = 0;
        for part in &parts {
            if part.text.trim().is_empty() {
                continue;
            }
            let mut attempts = 0;
            loop {
                self.limiter.wait(&*self.sleeper);
                match self
                    .client
                    .send_message(chat_id, thread_id, &part.text, &part.entities)
                {
                    Ok(()) => {
                        sent += 1;
                        break;
                    }
                    Err(
                        error @ TelegramError::Api {
                            code: Some(429),
                            retry_after,
                            ..
                        },
                    ) => {
                        if attempts >= 2 {
                            return Err(error);
                        }
                        self.sleeper
                            .sleep(Duration::from_secs(retry_after.unwrap_or(1)));
                        attempts += 1;
                    }
                    Err(TelegramError::Api {
                        code: Some(500..=599),
                        ..
                    }) => {
                        return Ok(SendOutcome::Uncertain {
                            message: "the Telegram server failed after the request".to_owned(),
                        });
                    }
                    Err(TelegramError::Transport(message)) => {
                        return Ok(SendOutcome::Uncertain {
                            message: format!("delivery is uncertain: {message}"),
                        });
                    }
                    Err(TelegramError::Protocol { .. }) => {
                        return Ok(SendOutcome::Uncertain {
                            message: "delivery is uncertain: invalid Telegram response".to_owned(),
                        });
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(SendOutcome::Fallback { parts: sent })
    }
}

fn rich_fits(markdown: &str) -> bool {
    markdown.len() <= RICH_TEXT_LIMIT
        && markdown
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            <= RICH_BLOCK_LIMIT
}

#[derive(Clone)]
struct RateLimiter {
    next: Arc<Mutex<Instant>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            next: Arc::new(Mutex::new(Instant::now())),
        }
    }

    fn wait(&self, sleeper: &dyn Sleeper) {
        let now = Instant::now();
        if let Ok(mut next) = self.next.lock() {
            if *next > now {
                sleeper.sleep(SEND_INTERVAL);
            }
            *next = Instant::now() + SEND_INTERVAL;
        }
    }
}
