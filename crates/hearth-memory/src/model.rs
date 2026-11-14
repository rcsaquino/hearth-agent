//! What a memory is.
//!
//! One row is one statement the user would recognise as a single thing they told
//! Hearth. The `kind` selects the retrieval prior and the digest wording. It
//! never selects a different store: rules, facts, and transcripts are different
//! shapes of evidence, but every statement about the user's world is one row in
//! one table with one index.
//!
//! Two fields carry the design:
//!
//! * `subject` is the normalised entity the statement is about. It is the
//!   strongest retrieval signal personal memory has, because a personal question
//!   almost always names a person, project, or thing.
//! * `valid_to` is set only when a newer statement replaces this one. A
//!   contradiction therefore becomes a date range rather than an overwrite, so
//!   "what is true now" and "what did we think in March" are both answerable.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::errors::{MemoryError, Result};

/// The kinds Hearth distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An instruction that must be obeyed without a retrieval cue.
    Rule,
    /// How the user wants something done.
    Preference,
    /// A person, with their relations and attributes.
    Person,
    /// A choice that was made, and why.
    Decision,
    /// Something the user agreed to do.
    Commitment,
    /// Something that happened, at a time.
    Episode,
    /// Everything else that is simply true.
    Fact,
}

impl Kind {
    pub const ALL: [Self; 7] = [
        Self::Rule,
        Self::Preference,
        Self::Person,
        Self::Decision,
        Self::Commitment,
        Self::Episode,
        Self::Fact,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Preference => "preference",
            Self::Person => "person",
            Self::Decision => "decision",
            Self::Commitment => "commitment",
            Self::Episode => "episode",
            Self::Fact => "fact",
        }
    }

    /// How much a kind is worth in a recall slot.
    ///
    /// A rule already sits in the prompt, so a slot spent on one repeats what the
    /// model has. Preferences and people rank highest because they are the
    /// answers a keyword search misses most often.
    pub fn prior(self) -> f32 {
        match self {
            Self::Preference => 1.0,
            Self::Person => 0.95,
            Self::Decision => 0.9,
            Self::Commitment => 0.85,
            Self::Fact => 0.75,
            Self::Episode => 0.55,
            Self::Rule => 0.4,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Kind {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value.trim().to_ascii_lowercase())
            .ok_or_else(|| {
                MemoryError::invalid(format!(
                    "kind must be one of {}.",
                    Self::ALL.map(Kind::as_str).join(", ")
                ))
            })
    }
}

/// Where a memory came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The user stated it.
    User,
    /// The agent chose to record it.
    Agent,
    /// The deterministic extractor caught it.
    Extract,
    /// It arrived from a file or an import.
    Import,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Extract => "extract",
            Self::Import => "import",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "agent" => Self::Agent,
            "extract" => Self::Extract,
            "import" => Self::Import,
            _ => Self::User,
        }
    }
}

/// One remembered statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub kind: Kind,
    pub content: String,
    pub subject: String,
    pub pinned: bool,
    pub source: Source,
    pub source_entry: Option<String>,
    pub valid_from: i64,
    pub valid_to: Option<i64>,
    pub superseded_by: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub revision: i64,
}

impl Memory {
    /// Whether this row is the current version of its statement.
    pub fn is_current(&self) -> bool {
        self.valid_to.is_none()
    }

    /// Whether this row was true at an instant, in microseconds.
    pub fn valid_at(&self, instant: i64) -> bool {
        self.valid_from <= instant && self.valid_to.is_none_or(|end| end > instant)
    }
}

/// A statement to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMemory {
    pub content: String,
    pub kind: Kind,
    pub subject: String,
    /// The row this statement replaces, if any.
    pub supersedes: Option<String>,
    pub pinned: bool,
    pub source: Source,
    pub source_entry: Option<String>,
    /// When the statement became true. Defaults to the write time.
    pub valid_from: Option<i64>,
    /// When the statement stopped being true, for an import of history.
    ///
    /// The tool surface never sets this, so the model cannot backdate a fact. It
    /// exists so a fixture or an import can state a window directly instead of
    /// replaying a supersession chain.
    pub valid_to: Option<i64>,
}

impl NewMemory {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            kind: Kind::Fact,
            subject: String::new(),
            supersedes: None,
            pinned: false,
            source: Source::Agent,
            source_entry: None,
            valid_from: None,
            valid_to: None,
        }
    }

    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }

    pub fn source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    pub fn from_entry(mut self, entry: Option<String>) -> Self {
        self.source_entry = entry;
        self
    }

    pub fn supersedes(mut self, id: impl Into<String>) -> Self {
        self.supersedes = Some(id.into());
        self
    }

    pub fn pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    pub fn valid_from(mut self, instant: i64) -> Self {
        self.valid_from = Some(instant);
        self
    }

    pub fn valid_to(mut self, instant: Option<i64>) -> Self {
        self.valid_to = instant;
        self
    }
}

/// What one write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// A new row was inserted.
    Created(Memory),
    /// The same statement already existed; the existing row is returned unchanged.
    Unchanged(Memory),
}

impl WriteOutcome {
    pub fn memory(&self) -> &Memory {
        match self {
            Self::Created(memory) | Self::Unchanged(memory) => memory,
        }
    }

    pub fn created(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}
