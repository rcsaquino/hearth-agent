//! Records and the save value object.

use serde_json::Value;

/// One chat and its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    pub parent_id: Option<String>,
    pub parent_entry_id: Option<String>,
    pub title: String,
    pub transport: String,
    pub transport_key: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
}

/// One permanent line in a conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub conversation_id: String,
    pub seq: i64,
    pub kind: String,
    pub content: Value,
    pub tokens_est: i64,
    pub source: Option<String>,
    pub prompt_hash: Option<String>,
    pub created_at: i64,
}

/// A small mutable JSON record attached to a conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub conversation_id: String,
    pub kind: String,
    pub version: u64,
    pub json: Value,
    pub updated_at: i64,
}

/// A message handed in with a receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    pub id: String,
    pub conversation_id: String,
    pub request_id: String,
    pub kind: String,
    pub content: Value,
    pub status: SubmissionStatus,
    pub answer_entry_id: Option<String>,
    pub error: Option<Value>,
    pub created_at: i64,
    pub settled_at: Option<i64>,
}

/// The ending of a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    Queued,
    Running,
    Done,
    Unanswered,
    Cancelled,
}

impl SubmissionStatus {
    /// The stored text for this status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Unanswered => "unanswered",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse a stored status.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "unanswered" => Some(Self::Unanswered),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// The outcome of one save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub seq: i64,
    pub entry_ids: Vec<String>,
}

/// One line in the save log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRecord {
    pub seq: i64,
    pub created_at: i64,
    pub conversation_id: Option<String>,
    pub task_id: Option<String>,
    pub kind: String,
}

/// The outcome of submitting a message.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmitResult {
    pub submission: Submission,
    pub created: bool,
}

/// An entry to append in a save.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEntry {
    pub kind: String,
    pub content: Value,
    pub source: Option<String>,
    pub prompt_hash: Option<String>,
}

/// A document change in a save.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentWrite {
    pub kind: String,
    pub expected: Option<u64>,
    pub json: Value,
}

/// A submission to insert in a save.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSubmission {
    pub id: String,
    pub request_id: String,
    pub kind: String,
    pub content: Value,
}

/// A submission ending in a save.
#[derive(Debug, Clone, PartialEq)]
pub struct Settle {
    pub submission_id: String,
    pub status: SubmissionStatus,
    pub answer_entry_id: Option<String>,
}

/// Everything that belongs to one change.
#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    pub(crate) conversation_id: String,
    pub(crate) entries: Vec<NewEntry>,
    pub(crate) documents: Vec<DocumentWrite>,
    pub(crate) submissions: Vec<NewSubmission>,
    pub(crate) settles: Vec<Settle>,
}

impl Save {
    /// Start a save for one conversation.
    pub fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            entries: Vec::new(),
            documents: Vec::new(),
            submissions: Vec::new(),
            settles: Vec::new(),
        }
    }

    /// Append an entry with simple content.
    pub fn entry(mut self, kind: impl Into<String>, content: Value) -> Self {
        self.entries.push(NewEntry {
            kind: kind.into(),
            content,
            source: None,
            prompt_hash: None,
        });
        self
    }

    /// Append a fully described entry.
    pub fn entry_with(mut self, entry: NewEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Write a document. `None` creates it; `Some(version)` updates that version.
    pub fn document(mut self, kind: impl Into<String>, expected: Option<u64>, json: Value) -> Self {
        self.documents.push(DocumentWrite {
            kind: kind.into(),
            expected,
            json,
        });
        self
    }

    /// Insert a submission.
    pub(crate) fn submission(mut self, submission: NewSubmission) -> Self {
        self.submissions.push(submission);
        self
    }

    /// Settle an existing submission.
    pub fn settle(
        mut self,
        submission_id: impl Into<String>,
        status: SubmissionStatus,
        answer_entry_id: Option<String>,
    ) -> Self {
        self.settles.push(Settle {
            submission_id: submission_id.into(),
            status,
            answer_entry_id,
        });
        self
    }
}
