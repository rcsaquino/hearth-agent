//! Stable error codes, ported from `src/errors.ts`.
//!
//! The original distinguishes expected, recoverable failures from unexpected
//! ones so a caller can act. This keeps the same codes and the same rule:
//! unexpected errors pass through untouched rather than being flattened.

use std::fmt;

/// Stable codes for expected, recoverable failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The caller supplied a value outside the accepted shape or range.
    InvalidInput,
    /// The referenced id does not exist.
    NotFound,
    /// A revision or expected-content guard rejected a stale write.
    Conflict,
    /// A standing edit would have duplicated another standing bullet.
    DuplicateStandingContent,
    /// The standing file cannot hold the requested bullet, even after eviction.
    StandingCapacity,
    /// SQLite could not be opened or read.
    StoreUnavailable,
    /// Another process holds the `standing.md` lock.
    StandingLocked,
}

impl ErrorCode {
    /// The wire spelling used by the original.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::DuplicateStandingContent => "duplicate_standing_content",
            Self::StandingCapacity => "standing_capacity",
            Self::StoreUnavailable => "store_unavailable",
            Self::StandingLocked => "standing_locked",
        }
    }
}

/// A failure with a stable code and structured details.
#[derive(Debug, Clone)]
pub struct ExpectedError {
    pub code: ErrorCode,
    pub message: String,
    /// Structured context a caller can branch on: field names, ids, revisions,
    /// and the capacity diagnostics the original returns on refusal.
    pub details: serde_json::Value,
}

impl ExpectedError {
    /// Build an error with no structured details.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: serde_json::Value::Null,
        }
    }

    /// Build an error carrying structured details.
    pub fn with_details(
        code: ErrorCode,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }

    /// The error code as a string.
    pub fn code(&self) -> &'static str {
        self.code.as_str()
    }
}

impl fmt::Display for ExpectedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ExpectedError {}

/// The crate result type: an expected failure or a store failure.
pub type Result<T> = std::result::Result<T, ExpectedError>;

impl From<rusqlite::Error> for ExpectedError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(
            ErrorCode::StoreUnavailable,
            format!("SQLite is unavailable: {error}"),
        )
    }
}

impl From<std::io::Error> for ExpectedError {
    fn from(error: std::io::Error) -> Self {
        Self::new(
            ErrorCode::StoreUnavailable,
            format!("filesystem is unavailable: {error}"),
        )
    }
}
