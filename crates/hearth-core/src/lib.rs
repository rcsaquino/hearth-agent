//! Hearth core: the permanent record, saves, submissions, and tasks.

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
mod zone;

pub use budget::{ContextBudget, TokenCounts};
pub use clock::{Clock, IdGen, SystemClock, UuidV7};
pub use compaction::{
    CompactionPlan, plan as plan_compaction, summary_content, summary_first_kept, summary_text,
};
pub use error::CoreError;
pub use guidance::{
    AGENTS_GUIDANCE_FILE, Guidance, GuidanceFile, HEARTH_GUIDANCE_FILE, load as load_guidance,
};
pub use model::{
    Commit, CommitRecord, Conversation, Effect, EffectIntent, EffectStatus, Entry, NewEntry,
    NewSubmission, NewUsage, PromptBlob, Save, Settle, Submission, SubmissionStatus, SubmitResult,
    TaskCounts, TelegramAdmission, TelegramUpdate, UsageEvent, UsageTotals, cache_hit_rate,
};
pub use prompt::{Prompt, PromptInput, build as build_prompt, parse_blocks, prefix_cause};
pub use session::{Harness, PROMPT_BLOB_RETENTION_MICROS, Session};
pub use soul::default_soul;
pub use task::{
    ClaimedTask, NewTask, RecoveryReport, StepOutput, Task, TaskStatus, TerminalOutcome,
};
pub use zone::{Local, Zone};
