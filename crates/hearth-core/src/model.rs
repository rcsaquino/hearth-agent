//! Records and the save value object.

use hearth_model::Usage;
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

    /// Whether this is one of the final endings.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Unanswered | Self::Cancelled)
    }
}

/// The state of an outside action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectStatus {
    Intent,
    Started,
    Done,
    Failed,
    Unknown,
}

impl EffectStatus {
    /// The stored text for this status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Started => "started",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    /// Parse a stored status.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "intent" => Some(Self::Intent),
            "started" => Some(Self::Started),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    /// Whether this status is final.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Unknown)
    }
}

/// One recorded action outside the database.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub id: String,
    pub task_id: Option<String>,
    pub kind: String,
    pub key: String,
    pub payload: Value,
    pub status: EffectStatus,
    pub result: Option<Value>,
    pub attempts: i64,
    pub created_at: i64,
    pub settled_at: Option<i64>,
}

/// An outside action to record in a save.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectIntent {
    pub kind: String,
    pub key: String,
    pub payload: Value,
    pub task_id: Option<String>,
}

impl EffectIntent {
    /// Create an effect intent with a stable key.
    pub fn new(kind: impl Into<String>, key: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            key: key.into(),
            payload,
            task_id: None,
        }
    }

    /// Attach the effect to a task.
    pub fn task(mut self, id: impl Into<String>) -> Self {
        self.task_id = Some(id.into());
        self
    }
}

/// The outcome of one save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub seq: i64,
    pub entry_ids: Vec<String>,
    pub effect_ids: Vec<String>,
}

/// A model call to record in the usage ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct NewUsage {
    pub task_id: Option<String>,
    pub conversation_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub kind: String,
    pub usage: Usage,
    pub cost_micros: i64,
    pub latency_ms: i64,
}

/// One recorded model call.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageEvent {
    pub id: String,
    pub ts: i64,
    pub task_id: Option<String>,
    pub conversation_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub kind: String,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_micros: i64,
    pub latency_ms: i64,
}

/// One saved prompt text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBlob {
    pub hash: String,
    pub text: String,
    pub created_at: i64,
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

/// One Telegram update admitted with a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub chat_id: i64,
    pub message_id: i64,
}

/// The outcome of a Telegram admission attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct TelegramAdmission {
    pub submission: Submission,
    pub duplicate: bool,
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
    pub error: Option<Value>,
}

/// Everything that belongs to one change.
#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    pub(crate) conversation_id: String,
    pub(crate) entries: Vec<NewEntry>,
    pub(crate) documents: Vec<DocumentWrite>,
    pub(crate) submissions: Vec<NewSubmission>,
    pub(crate) settles: Vec<Settle>,
    pub(crate) effects: Vec<EffectIntent>,
    pub(crate) telegram_update: Option<TelegramUpdate>,
    pub(crate) telegram_offset: Option<i64>,
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
            effects: Vec::new(),
            telegram_update: None,
            telegram_offset: None,
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
        self,
        submission_id: impl Into<String>,
        status: SubmissionStatus,
        answer_entry_id: Option<String>,
    ) -> Self {
        self.settle_with_error(submission_id, status, answer_entry_id, None)
    }

    /// Settle a submission and record an optional structured reason.
    pub fn settle_with_error(
        mut self,
        submission_id: impl Into<String>,
        status: SubmissionStatus,
        answer_entry_id: Option<String>,
        error: Option<Value>,
    ) -> Self {
        self.settles.push(Settle {
            submission_id: submission_id.into(),
            status,
            answer_entry_id,
            error,
        });
        self
    }

    /// Record an outside action that should happen after the save.
    pub fn effect(mut self, intent: EffectIntent) -> Self {
        self.effects.push(intent);
        self
    }

    /// Record the Telegram update admitted by this save.
    pub fn telegram_update(mut self, update: TelegramUpdate) -> Self {
        self.telegram_update = Some(update);
        self
    }

    /// Advance the Telegram offset in the same save.
    pub fn telegram_offset(mut self, offset: i64) -> Self {
        self.telegram_offset = Some(offset);
        self
    }
}
