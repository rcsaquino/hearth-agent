//! Structured errors for the durable core.

use hearth_storage::StorageError;
use thiserror::Error;

/// A failure in the permanent record.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("conversation {0} not found")]
    ConversationNotFound(String),
    #[error("entry {0} not found in conversation {1}")]
    EntryNotFound(String, String),
    #[error("submission {0} not found")]
    SubmissionNotFound(String),
    #[error("document {kind} version mismatch: expected {expected:?}, found {found:?}")]
    DocumentVersion {
        kind: String,
        expected: Option<u64>,
        found: Option<u64>,
    },
    #[error("submission {0} is already settled as {1}")]
    AlreadySettled(String, String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("stored data is invalid: {0}")]
    Corrupt(String),
}
