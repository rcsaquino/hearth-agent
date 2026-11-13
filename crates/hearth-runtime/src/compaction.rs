//! Compaction: summarize an old range into one summary entry.

use std::sync::Arc;
use std::time::Instant;

use hearth_core::{
    CompactionPlan, CoreError, Entry, NewEntry, NewTask, NewUsage, Save, Session, StepOutput,
    TerminalOutcome, parse_blocks, summary_content,
};
use hearth_model::{ChatMessage, ChatRequest, ContentBlock, Provider, Role, StreamEvent, Usage};
use serde_json::json;
use thiserror::Error;

const ENTRY_PREVIEW_CHARS: usize = 400;
const RANGE_CHARS_PER_ENTRY: usize = 1200;

const SUMMARY_INSTRUCTION: &str = "You summarize an older part of a conversation so it can be \
replaced by your summary. Preserve decisions, open threads, names, dates, numbers, and user \
preferences. Drop small talk and superseded detail. Return only the summary text, with no \
preamble and no JSON.";

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
        }
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

        let summary = text.trim();
        if summary.is_empty() {
            self.fail_task(&claimed.id, "the summary was empty")?;
            return Err(CompactionError::Invalid("the summary was empty".to_owned()));
        }
        let entry = NewEntry {
            kind: "summary".to_owned(),
            content: summary_content(summary, plan),
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
