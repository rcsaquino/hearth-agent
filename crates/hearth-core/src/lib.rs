//! Hearth core: the permanent record, saves, documents, submissions, and forks.

mod budget;
mod clock;
mod compaction;
mod error;
mod guidance;
mod model;
mod prompt;
pub mod secrets;
mod session;
mod soul;
mod task;

pub use budget::{
    ContextBudget, FIXED_OVERHEAD_TARGET, SEGMENT_IDENTITY, SEGMENT_OPERATION, SEGMENT_RECALL,
    SEGMENT_RUNTIME, SEGMENT_SKILLS, SEGMENT_STANDING, SEGMENT_TOOLS, TokenCounts,
};
pub use clock::{Clock, IdGen, SystemClock, UuidV7};
pub use compaction::{
    CompactionPlan, plan as plan_compaction, summary_content, summary_first_kept, summary_text,
};
pub use error::CoreError;
pub use guidance::{
    AGENTS_GUIDANCE_FILE, Guidance, HEARTH_GUIDANCE_FILE, MAX_GUIDANCE_FILE_BYTES,
    MAX_GUIDANCE_TOTAL_BYTES, load as load_guidance,
};
pub use model::{
    Commit, CommitRecord, Conversation, Document, DocumentWrite, Effect, EffectIntent,
    EffectStatus, Entry, MemoryChange, MemoryUpdate, MemoryWrite, NewEntry, NewMemory,
    NewSubmission, NewUsage, PromptBlob, Save, Settle, Submission, SubmissionStatus, SubmitResult,
    TaskCounts, TelegramAdmission, TelegramUpdate, UsageEvent, UsageTotals,
};
pub use prompt::{
    Prompt, PromptInput, build as build_prompt, parse_blocks, prefix_cause, request_from_document,
};
pub use session::{Harness, Session};
pub use soul::default_soul;
pub use task::{
    ClaimedTask, NewTask, RecoveryReport, StepOutput, Task, TaskStatus, TerminalOutcome,
};
