//! The post-turn memory net.
//!
//! The agent's own `memory_save` call is the primary write path. This runs after
//! the answer is delivered and catches the four phrasings where the user's
//! durable intent is not in doubt, at a cost of a few microseconds and no tokens.
//!
//! It deliberately sees only the user's message, never the answer. A model's own
//! prose is not evidence of what the user said, and extracting from it is how a
//! store fills with restatements of the conversation.

use hearth_core::Session;
use hearth_memory::MemoryService;

/// What one capture pass wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureReport {
    pub written: usize,
    pub skipped: usize,
}

/// Capture the durable statements in one user message.
///
/// A failure is logged by the caller and never reaches the user: memory is a
/// convenience, and the answer has already been delivered.
pub fn capture(
    memory: &MemoryService,
    entry: Option<String>,
    text: &str,
) -> hearth_memory::Result<CaptureReport> {
    let candidates = hearth_memory::candidates(text);
    let mut report = CaptureReport::default();
    for candidate in candidates {
        match memory.write(candidate.into_new(entry.clone())) {
            Ok(outcome) if outcome.created() => report.written += 1,
            Ok(_) => report.skipped += 1,
            Err(error) if error.code == hearth_memory::Code::Credential => report.skipped += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

/// Capture from one user entry the run answered.
///
/// The entry id comes from the submission, not from the newest row in the
/// history, so a queued message is captured by its own run and a replay of the
/// same entry writes nothing.
pub fn capture_entry(
    session: &Session,
    memory: &MemoryService,
    entry_id: &str,
) -> hearth_memory::Result<CaptureReport> {
    let Some(entry) = session
        .entry(entry_id)
        .map_err(|error| hearth_memory::MemoryError::unavailable(error.to_string()))?
    else {
        return Ok(CaptureReport::default());
    };
    if entry.kind != "user" {
        return Ok(CaptureReport::default());
    }
    let text = plain_text(&entry.content);
    capture(memory, Some(entry.id.clone()), &text)
}

/// Flatten stored entry content into plain prose.
fn plain_text(content: &serde_json::Value) -> String {
    hearth_core::parse_blocks(content)
        .into_iter()
        .filter_map(|block| match block {
            hearth_model::ContentBlock::Text(text) => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

