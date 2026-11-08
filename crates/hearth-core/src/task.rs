//! Tasks that can pause, survive a restart, and continue.

use serde_json::Value;

/// The state of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Asleep,
    Done,
    Failed,
    Aborted,
}

impl TaskStatus {
    /// The stored text for this status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Asleep => "asleep",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }

    /// Parse a stored status.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "asleep" => Some(Self::Asleep),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "aborted" => Some(Self::Aborted),
            _ => None,
        }
    }

    /// Whether this status is final.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Aborted)
    }
}

/// A piece of work that can pause and resume.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub conversation_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub kind: String,
    pub version: i64,
    pub status: TaskStatus,
    pub phase: String,
    pub attempt: i64,
    pub input: Value,
    pub checkpoint: Option<Value>,
    pub result: Option<Value>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<i64>,
    pub next_wake_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub terminal_at: Option<i64>,
}

/// A task to insert.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTask {
    pub kind: String,
    pub version: i64,
    pub phase: String,
    pub input: Value,
    pub conversation_id: Option<String>,
    pub parent_task_id: Option<String>,
}

impl NewTask {
    /// Start a new task at its first phase.
    pub fn new(
        kind: impl Into<String>,
        version: i64,
        phase: impl Into<String>,
        input: Value,
    ) -> Self {
        Self {
            kind: kind.into(),
            version,
            phase: phase.into(),
            input,
            conversation_id: None,
            parent_task_id: None,
        }
    }

    /// Attach the task to a conversation.
    pub fn conversation(mut self, id: impl Into<String>) -> Self {
        self.conversation_id = Some(id.into());
        self
    }

    /// Attach the task to a parent task.
    pub fn parent(mut self, id: impl Into<String>) -> Self {
        self.parent_task_id = Some(id.into());
        self
    }
}

/// The view of a task handed to one worker after a claim.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedTask {
    pub id: String,
    pub conversation_id: Option<String>,
    pub kind: String,
    pub phase: String,
    pub attempt: i64,
    pub input: Value,
    pub checkpoint: Option<Value>,
}

/// What one step decided.
#[derive(Debug, Clone, PartialEq)]
pub enum StepOutput {
    Next {
        phase: String,
        checkpoint: Value,
    },
    Sleep {
        until_micros: i64,
        phase: String,
        checkpoint: Value,
    },
    Terminal {
        outcome: TerminalOutcome,
        result: Value,
    },
    Failed {
        error: Value,
        retryable: bool,
    },
}

/// The final outcome of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutcome {
    Done,
    Failed,
    Aborted,
}

impl TerminalOutcome {
    /// The task status for this outcome.
    pub fn status(self) -> TaskStatus {
        match self {
            Self::Done => TaskStatus::Done,
            Self::Failed => TaskStatus::Failed,
            Self::Aborted => TaskStatus::Aborted,
        }
    }
}

/// What recovery changed on startup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub requeued: u64,
    pub poisoned: u64,
    pub woken: u64,
}
