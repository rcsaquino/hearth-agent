//! Provider boundary: requests, streams, and errors.

use std::time::Duration;

use thiserror::Error;

use crate::types::{ChatRequest, StreamEvent};

/// A streaming provider response.
pub type ProviderStream = Box<dyn Iterator<Item = Result<StreamEvent, ProviderError>> + Send>;

/// A model service.
pub trait Provider: Send + Sync {
    /// Start one streaming request.
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError>;
}

/// A blocking delay, injected so retry tests do not wait.
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

/// A provider failure.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider configuration error: {0}")]
    Config(String),
    #[error("provider transport error: {0}")]
    Transport(String),
    #[error("provider returned {status}: {message}")]
    Request {
        status: u16,
        message: String,
        retryable: bool,
    },
    #[error("provider protocol error: {0}")]
    Protocol(String),
}

impl ProviderError {
    /// Whether the same request may succeed later.
    pub fn retryable(&self) -> bool {
        match self {
            Self::Request { retryable, .. } => *retryable,
            Self::Transport(_) => true,
            Self::Config(_) | Self::Protocol(_) => false,
        }
    }

    /// The HTTP status when the provider answered.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Request { status, .. } => Some(*status),
            _ => None,
        }
    }
}
