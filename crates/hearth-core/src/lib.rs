//! Hearth core: the permanent record, saves, documents, submissions, and forks.

mod budget;
mod clock;
mod error;
mod model;
mod prompt;
mod session;
mod soul;
mod task;

pub use budget::{
    ContextBudget, FIXED_OVERHEAD_TARGET, SEGMENT_IDENTITY, SEGMENT_OPERATION, SEGMENT_RECALL,
    SEGMENT_RUNTIME, SEGMENT_SKILLS, SEGMENT_STANDING, SEGMENT_TOOLS, TokenCounts,
};
pub use clock::{Clock, IdGen, SystemClock, UuidV7};
pub use error::CoreError;
pub use model::{
    Commit, CommitRecord, Conversation, Document, DocumentWrite, Effect, EffectIntent,
    EffectStatus, Entry, MemoryChange, MemoryUpdate, MemoryWrite, NewEntry, NewMemory,
    NewSubmission, NewUsage, PromptBlob, Save, Settle, Submission, SubmissionStatus, SubmitResult,
    TaskCounts, TelegramAdmission, TelegramUpdate, UsageEvent, UsageTotals,
};
pub use prompt::{Prompt, PromptInput, build as build_prompt, parse_blocks};
pub use session::{Harness, Session};
pub use soul::default_soul;
pub use task::{
    ClaimedTask, NewTask, RecoveryReport, StepOutput, Task, TaskStatus, TerminalOutcome,
};
