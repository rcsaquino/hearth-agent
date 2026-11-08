//! Memory records, enums, and errors.

use hearth_core::CoreError;
use hearth_storage::StorageError;
use thiserror::Error;

/// One kind of long-term memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryKind {
    Fact,
    Preference,
    Person,
    Project,
    Event,
    Episode,
    Rule,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Person => "person",
            Self::Project => "project",
            Self::Event => "event",
            Self::Episode => "episode",
            Self::Rule => "rule",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fact" => Some(Self::Fact),
            "preference" => Some(Self::Preference),
            "person" => Some(Self::Person),
            "project" => Some(Self::Project),
            "event" => Some(Self::Event),
            "episode" => Some(Self::Episode),
            "rule" => Some(Self::Rule),
            _ => None,
        }
    }
}

/// The lifecycle state of a memory row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryStatus {
    Active,
    Superseded,
    Archived,
    Deleted,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "archived" => Some(Self::Archived),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

/// Where a memory came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    User,
    Assistant,
    Session,
    External,
    Inference,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Session => "session",
            Self::External => "external",
            Self::Inference => "inference",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "session" => Some(Self::Session),
            "external" => Some(Self::External),
            "inference" => Some(Self::Inference),
            _ => None,
        }
    }
}

/// Whether a memory is always shown or promoted to standing memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    None,
    Promoted,
    Pinned,
}

impl Standing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Promoted => "promoted",
            Self::Pinned => "pinned",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "promoted" => Some(Self::Promoted),
            "pinned" => Some(Self::Pinned),
            _ => None,
        }
    }
}

/// One row in the memory library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryItem {
    pub id: String,
    pub kind: MemoryKind,
    pub content: String,
    pub tags: Vec<String>,
    pub fingerprint: String,
    pub importance: i64,
    pub status: MemoryStatus,
    pub valid_from: Option<i64>,
    pub valid_to: Option<i64>,
    pub supersedes_id: Option<String>,
    pub source_kind: SourceKind,
    pub source_ref: Option<String>,
    pub standing: Standing,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// The current standing-memory block and its version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingMemory {
    pub items: Vec<MemoryItem>,
    pub version: u64,
}

/// Counts for the memory tool's stats action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryStats {
    pub total: i64,
    pub by_status: std::collections::BTreeMap<String, i64>,
    pub by_kind: std::collections::BTreeMap<String, i64>,
}

/// The result of one consolidation pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsolidationReport {
    pub merged: usize,
    pub unchanged: usize,
}

/// A memory failure.
#[derive(Debug, Error)]
pub enum MemoryError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("memory {0} not found")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("content looks like a credential and was not stored: {0}")]
    Secret(String),
    #[error("stored memory is invalid: {0}")]
    Corrupt(String),
}
