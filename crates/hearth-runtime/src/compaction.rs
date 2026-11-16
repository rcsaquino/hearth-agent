//! Compaction: summarize an old range into one summary entry.

use std::sync::Arc;
use std::time::Instant;

use hearth_core::{
    CompactionPlan, CoreError, Entry, NewEntry, NewTask, NewUsage, Save, Session, StepOutput,
    TerminalOutcome, parse_blocks, summary_content,
};
use hearth_memory::{Kind, MemoryService, NewMemory, Source};
use hearth_model::{ChatMessage, ChatRequest, ContentBlock, Provider, Role, StreamEvent, Usage};
use serde_json::json;
use thiserror::Error;

const ENTRY_PREVIEW_CHARS: usize = 400;
const RANGE_CHARS_PER_ENTRY: usize = 1200;
/// The most memories one summary may carry.
const MAX_SUMMARY_MEMORIES: usize = 3;

/// One call returns both the summary and the durable statements in the range.
///
/// Folding the two used to cost a second request over the same text for every
/// compaction. The model already has the range in front of it, so asking for the
/// memories in the same answer costs a few dozen output tokens instead of a whole
/// second prompt.
const SUMMARY_INSTRUCTION: &str = "You compress an older part of a conversation so it \
can be replaced by your output. Return one JSON object with two keys and nothing else. \
`summary` is a compact paragraph that keeps decisions, open threads, names, dates, numbers, \
and user preferences, and drops small talk and superseded detail. `memories` is a list of at \
most three durable statements from the range, each a JSON object with `content` (one statement), \
`kind` (one of fact, preference, person, decision, commitment, episode), and `subject` (the \
person, project, or thing it is about). Use an empty list when nothing in the range is worth \
recalling later. Do not record requests, greetings, tool output, or credentials.";

/// A failure that stopped a compaction.
#[derive(Debug, Error)]
pub enum CompactionError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("{0}")]
    Invalid(String),
}

/// Summarizes one planned range into a single summary entry.
pub struct Compactor {
    session: Session,
    provider: Arc<dyn Provider>,
    model: String,
    provider_name: String,
    owner: String,
    memory: Option<std::sync::Arc<MemoryService>>,
}

impl Compactor {
    /// Build a compactor.
    pub fn new(
        session: Session,
        provider: Arc<dyn Provider>,
        model: String,
        provider_name: String,
        owner: String,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            provider_name,
            owner,
            memory: None,
        }
    }

    /// Write the durable statements the summary pass finds, in the same save.
    pub fn with_memory(mut self, memory: std::sync::Arc<MemoryService>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Summarize the planned range and save one summary entry.
    pub fn compact(
        &self,
        conversation_id: &str,
        plan: &CompactionPlan,
    ) -> Result<Option<Entry>, CompactionError> {
        let task = self.session.create_task(
            NewTask::new(
                "compaction",
                1,
                "summarize",
                json!({
                    "conversation_id": conversation_id,
                    "first_seq": plan.first_seq,
                    "last_seq": plan.last_seq,
                    "first_kept": plan.first_kept,
                    "entry_ids": plan.entries.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>(),
                }),
            )
            .conversation(conversation_id),
        )?;
        let claimed = self
            .session
            .claim_task_by_id(&task.id, &self.owner, 60_000_000)?
            .ok_or_else(|| {
                CompactionError::Invalid("the compaction task could not be claimed".to_owned())
            })?;

        let instruction = SUMMARY_INSTRUCTION.to_owned();
        let range = render_range(&plan.entries);
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    blocks: vec![ContentBlock::Text(instruction.clone())],
                },
                ChatMessage {
                    role: Role::User,
                    blocks: vec![ContentBlock::Text(range.clone())],
                },
            ],
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            session_id: Some(conversation_id.to_owned()),
        };
        let prompt_text = format!("{instruction}\n\n{range}");
        let prompt_hash = self.session.save_prompt_blob(&prompt_text)?;

        let started = Instant::now();
        let stream = match self.provider.stream(request) {
            Ok(stream) => stream,
            Err(error) => {
                self.fail_task(&claimed.id, &format!("provider error: {error}"))?;
                return Err(CompactionError::Provider(error.to_string()));
            }
        };
        let mut text = String::new();
        let mut usage = Usage::default();
        for event in stream {
            match event {
                Ok(StreamEvent::TextDelta(delta)) => text.push_str(&delta),
                Ok(StreamEvent::Usage(value)) => usage = value,
                Ok(_) => {}
                Err(error) => {
                    let latency_ms = started.elapsed().as_millis() as i64;
                    if has_usage(usage) {
                        let _ = self.record_usage(&claimed.id, conversation_id, usage, latency_ms);
                    }
                    self.fail_task(&claimed.id, &format!("provider error: {error}"))?;
                    return Err(CompactionError::Provider(error.to_string()));
                }
            }
        }
        let latency_ms = started.elapsed().as_millis() as i64;
        self.record_usage(&claimed.id, conversation_id, usage, latency_ms)?;

        let (summary, memories) = parse_summary(&text);
        if summary.is_empty() {
            self.fail_task(&claimed.id, "the summary was empty")?;
            return Err(CompactionError::Invalid("the summary was empty".to_owned()));
        }
        if let Some(memory) = &self.memory {
            let source = plan.entries.last().map(|entry| entry.id.clone());
            for candidate in memories.into_iter().take(MAX_SUMMARY_MEMORIES) {
                let item = candidate
                    .into_new()
                    .source(Source::Extract)
                    .from_entry(source.clone());
                if let Err(error) = memory.write(item) {
                    tracing::warn!(%error, "a compacted memory was not written");
                }
            }
        }
        let entry = NewEntry {
            kind: "summary".to_owned(),
            content: summary_content(&summary, plan),
            source: None,
            prompt_hash: Some(prompt_hash),
        };
        let commit = self
            .session
            .save(Save::new(conversation_id).entry_with(entry))?;
        let summary_id = commit
            .entry_ids
            .first()
            .cloned()
            .ok_or_else(|| CompactionError::Invalid("the summary was not saved".to_owned()))?;
        self.session.finish_task(
            &claimed.id,
            &self.owner,
            StepOutput::Terminal {
                outcome: TerminalOutcome::Done,
                result: json!({"summary_entry_id": summary_id}),
            },
        )?;
        Ok(self
            .session
            .history(conversation_id)?
            .into_iter()
            .find(|entry| entry.id == summary_id))
    }

    fn record_usage(
        &self,
        task_id: &str,
        conversation_id: &str,
        usage: Usage,
        latency_ms: i64,
    ) -> Result<(), CompactionError> {
        self.session.record_usage(NewUsage {
            task_id: Some(task_id.to_owned()),
            conversation_id: Some(conversation_id.to_owned()),
            provider: self.provider_name.clone(),
            model: self.model.clone(),
            kind: "compaction".to_owned(),
            usage,
            latency_ms,
        })?;
        Ok(())
    }

    fn fail_task(&self, task_id: &str, message: &str) -> Result<(), CoreError> {
        self.session.finish_task(
            task_id,
            &self.owner,
            StepOutput::Terminal {
                outcome: TerminalOutcome::Failed,
                result: json!({"error": short(message)}),
            },
        )?;
        Ok(())
    }
}

/// One durable statement a summary pass decided to keep.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMemory {
    content: String,
    kind: Kind,
    subject: String,
}

impl ParsedMemory {
    fn into_new(self) -> NewMemory {
        NewMemory::new(self.content)
            .kind(self.kind)
            .subject(self.subject)
    }
}

/// Read the summary and the memories out of one answer.
///
/// A model that ignores the format must not lose the summary, so anything that
/// does not parse is taken at face value as prose and the memories are dropped.
fn parse_summary(text: &str) -> (String, Vec<ParsedMemory>) {
    let trimmed = text.trim();
    let Some(start) = trimmed.find('{') else {
        return (trimmed.to_owned(), Vec::new());
    };
    let Some(end) = trimmed.rfind('}') else {
        return (trimmed.to_owned(), Vec::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]) else {
        return (trimmed.to_owned(), Vec::new());
    };
    let summary = value
        .get("summary")
        .and_then(|summary| summary.as_str())
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| trimmed.to_owned());
    let memories = value
        .get("memories")
        .and_then(|memories| memories.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let content = item.get("content")?.as_str()?.trim();
                    if content.is_empty() {
                        return None;
                    }
                    let kind = item
                        .get("kind")
                        .and_then(|kind| kind.as_str())
                        .and_then(|kind| kind.parse().ok())
                        .unwrap_or(Kind::Fact);
                    let subject = item
                        .get("subject")
                        .and_then(|subject| subject.as_str())
                        .unwrap_or("")
                        .to_owned();
                    Some(ParsedMemory {
                        content: content.to_owned(),
                        kind,
                        subject,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (summary, memories)
}

fn render_range(entries: &[Entry]) -> String {
    let mut rendered = String::from("## Conversation range\n");
    for entry in entries {
        let text = entry_text(entry);
        if text.trim().is_empty() {
            continue;
        }
        let text: String = text.chars().take(RANGE_CHARS_PER_ENTRY).collect();
        rendered.push_str(&format!("[{}] {}: {}\n", entry.id, entry.kind, text));
    }
    rendered
}

fn entry_text(entry: &Entry) -> String {
    parse_blocks(&entry.content)
        .into_iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text),
            ContentBlock::Thinking(_) => None,
            ContentBlock::ToolCall(call) => {
                Some(format!("tool call {} {}", call.name, call.arguments))
            }
            ContentBlock::ToolResult { content, .. } => Some(format!(
                "tool result: {}",
                content
                    .chars()
                    .take(ENTRY_PREVIEW_CHARS)
                    .collect::<String>()
            )),
            ContentBlock::Image { path, .. } => Some(format!("[image {path}]")),
            ContentBlock::File { name, .. } => Some(format!("[file {name}]")),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_usage(usage: Usage) -> bool {
    usage.input_tokens > 0
        || usage.cache_read_tokens > 0
        || usage.cache_write_tokens > 0
        || usage.output_tokens > 0
        || usage.reasoning_tokens > 0
}

fn short(message: &str) -> String {
    let single = message.replace(['\n', '\r'], " ");
    let text: String = single.trim().chars().take(200).collect();
    if text.is_empty() {
        "compaction failed".to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_instruction_asks_for_a_named_actor() {
        assert!(
            SUMMARY_INSTRUCTION.contains("standalone"),
            "the compaction instruction must ask for a standalone statement: {SUMMARY_INSTRUCTION}"
        );
        assert!(
            SUMMARY_INSTRUCTION.contains("pronoun"),
            "the compaction instruction must forbid a pronoun: {SUMMARY_INSTRUCTION}"
        );
    }
}
