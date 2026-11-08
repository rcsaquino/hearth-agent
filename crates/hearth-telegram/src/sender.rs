//! Telegram reply delivery with a rich-first path.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::{TelegramClient, TelegramError};

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
    pub fn send_reply(&self, chat_id: i64, markdown: &str) -> Result<SendOutcome, TelegramError> {
        if rich_fits(markdown) {
            self.limiter.wait(&*self.sleeper);
            match self.client.send_rich_message(chat_id, markdown) {
                Ok(()) => return Ok(SendOutcome::Rich),
                Err(error) => return Err(error),
            }
        }
        Err(TelegramError::Transport(
            "the reply exceeds the rich message limit".to_owned(),
        ))
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

struct RateLimiter {
    next: Mutex<Instant>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            next: Mutex::new(Instant::now()),
        }
    }

    fn wait(&self, sleeper: &dyn Sleeper) {
        let now = Instant::now();
        if let Ok(mut next) = self.next.lock() {
            if *next > now {
                sleeper.sleep(*next - now);
            }
            *next = Instant::now() + SEND_INTERVAL;
        }
    }
}
