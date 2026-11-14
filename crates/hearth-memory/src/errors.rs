//! Failures a caller may see.
//!
//! Every message is one sentence and safe to show. A failure carries a code so a
//! tool result can say `invalid_input` without inventing prose, and so the daemon
//! can log a category without dumping content.

use std::fmt;

/// The category of a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// The caller sent something a memory write cannot accept.
    InvalidInput,
    /// A credential-shaped string was refused.
    Credential,
    /// The rules store is full and nothing was written.
    RulesFull,
    /// The named memory does not exist.
    NotFound,
    /// The store could not be read or written.
    Unavailable,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::Credential => "credential_refused",
            Self::RulesFull => "rules_full",
            Self::NotFound => "not_found",
            Self::Unavailable => "memory_unavailable",
        }
    }
}

/// A memory failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryError {
    pub code: Code,
    pub message: String,
}

impl MemoryError {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidInput, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(Code::Unavailable, message)
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MemoryError {}

impl From<hearth_storage::StorageError> for MemoryError {
    fn from(error: hearth_storage::StorageError) -> Self {
        Self::unavailable(format!("memory storage failed: {error}"))
    }
}

impl From<std::io::Error> for MemoryError {
    fn from(error: std::io::Error) -> Self {
        Self::unavailable(format!("memory file access failed: {error}"))
    }
}

impl From<rusqlite::Error> for MemoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::unavailable(format!("memory query failed: {error}"))
    }
}

pub type Result<T> = std::result::Result<T, MemoryError>;
