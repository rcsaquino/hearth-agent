//! Hearth core: the permanent record, saves, documents, submissions, and forks.

mod clock;
mod error;
mod model;
mod prompt;
mod session;
mod soul;
mod task;

pub use clock::{Clock, IdGen, SystemClock, UuidV7};
pub use error::CoreError;
pub use model::{
    Commit, CommitRecord, Conversation, Document, DocumentWrite, Effect, EffectIntent,
    EffectStatus, Entry, NewEntry, NewSubmission, NewUsage, PromptBlob, Save, Settle, Submission,
    SubmissionStatus, SubmitResult, TelegramAdmission, TelegramUpdate, UsageEvent,
};
pub use prompt::{Prompt, PromptInput, parse_blocks};
pub use session::{Harness, Session};
pub use soul::default_soul;
pub use task::{
    ClaimedTask, NewTask, RecoveryReport, StepOutput, Task, TaskStatus, TerminalOutcome,
};
