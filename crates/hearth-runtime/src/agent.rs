//! The agent loop: one submission, one durable generation task, one answer.

use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use hearth_core::{
    CoreError, Entry, NewEntry, NewTask, NewUsage, PromptInput, Save, Session, StepOutput,
    Submission, SubmissionStatus, TerminalOutcome, parse_blocks,
};
use hearth_model::{ContentBlock, Pricing, Provider, StreamEvent, ToolCall, Usage};
use hearth_tools::{ToolError, ToolRegistry};
use std::path::PathBuf;
use serde_json::{Value, json};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Everything an agent needs beyond the record and the provider.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub soul: String,
    pub model: String,
    pub provider_name: String,
    pub owner: String,
    pub max_turns: u32,
    pub tools: ToolRegistry,
    pub spill_dir: PathBuf,
}

/// The outcome of one submission.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub submission: Submission,
    pub task_id: String,
    pub answer_entry_id: Option<String>,
    pub answer_text: Option<String>,
    pub notice: Option<String>,
}

/// A failure that prevented the run from starting or finishing.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("another run is active in this conversation")]
    Busy,
    #[error("{0}")]
    Invalid(String),
}

/// Runs submissions against a provider.
pub struct Agent {
    session: Session,
    provider: Arc<dyn Provider>,
    pricing: Pricing,
    config: AgentConfig,
    reservations: Mutex<HashSet<String>>,
}

impl Agent {
    /// Build an agent.
    pub fn new(
        session: Session,
        provider: Arc<dyn Provider>,
        pricing: Pricing,
        config: AgentConfig,
    ) -> Self {
        Self {
            session,
            provider,
            pricing,
            config,
            reservations: Mutex::new(HashSet::new()),
        }
    }

    fn reserve(&self, conversation_id: &str) -> Result<Reservation<'_>, AgentError> {
        let mut reserved = self
            .reservations
            .lock()
            .map_err(|_| AgentError::Invalid("reservation lock is poisoned".to_owned()))?;
        if !reserved.insert(conversation_id.to_owned()) {
            return Err(AgentError::Busy);
        }
        drop(reserved);
        Ok(Reservation {
            reservations: &self.reservations,
            conversation_id: conversation_id.to_owned(),
        })
    }

    /// Run one submission to a final answer or a settled failure.
    pub fn run_submission(&self, submission_id: &str) -> Result<RunOutcome, AgentError> {
        let submission = self
            .session
            .submission_by_id(submission_id)?
            .ok_or_else(|| CoreError::SubmissionNotFound(submission_id.to_owned()))?;
        if submission.status.is_terminal() {
            return Err(AgentError::Invalid(
                "submission is already settled".to_owned(),
            ));
        }
        let _reservation = self.reserve(&submission.conversation_id)?;
        if self
            .session
            .active_generation_task(&submission.conversation_id)?
            .is_some()
        {
            return Err(AgentError::Busy);
        }

        let submission = self.session.begin_submission(submission_id)?;
        let task = self.session.create_task(
            NewTask::new(
                "generation",
                1,
                "request",
                json!({
                    "submission_id": submission.id,
                    "conversation_id": submission.conversation_id,
                }),
            )
            .conversation(&submission.conversation_id),
        )?;
        let claimed = self
            .session
            .claim_task_by_id(&task.id, &self.config.owner, 60_000_000)?
            .ok_or_else(|| {
                AgentError::Invalid("the generation task could not be claimed".to_owned())
            })?;
        self.session
            .heartbeat_task(&claimed.id, &self.config.owner, 60_000_000)?;

        let mut turn = claimed
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.get("turn"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        let history = self.session.history(&submission.conversation_id)?;
        let pending = pending_calls(&history);
        if !pending.is_empty() {
            self.execute_calls(&submission, &claimed.id, &pending)?;
            turn += 1;
            self.session.checkpoint_task(
                &claimed.id,
                &self.config.owner,
                json!({"turn": turn}),
            )?;
        }

        loop {
            if turn >= self.config.max_turns {
                return self.settle_failure(
                    &submission,
                    &claimed.id,
                    "The turn limit was reached before a final answer.",
                    "turn_limit",
                );
            }
            let now = OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| "unknown time".to_owned());
            let runtime_note = format!(
                "Current time: {now}. Conversation: {}. Model: {}.",
                submission.conversation_id, self.config.model
            );
            let prompt = self.session.build_prompt(PromptInput {
                soul: self.config.soul.clone(),
                conversation_id: submission.conversation_id.clone(),
                model: self.config.model.clone(),
                tools: self.config.tools.specs(),
                runtime_note,
            })?;
            let prompt_text = serde_json::to_string(&prompt.document)
                .map_err(|error| AgentError::Invalid(error.to_string()))?;
            let prompt_hash = self.session.save_prompt_blob(&prompt_text)?;
            self.session.checkpoint_task(
                &claimed.id,
                &self.config.owner,
                json!({"turn": turn, "prompt_hash": prompt_hash}),
            )?;

            let started = Instant::now();
            let stream = match self.provider.stream(prompt.request) {
                Ok(stream) => stream,
                Err(error) => {
                    return self.settle_failure(
                        &submission,
                        &claimed.id,
                        &format!("Provider error: {error}"),
                        "provider_error",
                    );
                }
            };
            let mut text = String::new();
            let mut thinking = String::new();
            let mut calls = Vec::new();
            let mut usage = Usage::default();
            for event in stream {
                match event {
                    Ok(StreamEvent::TextDelta(delta)) => text.push_str(&delta),
                    Ok(StreamEvent::ThinkingDelta(delta)) => thinking.push_str(&delta),
                    Ok(StreamEvent::ToolCall(call)) => calls.push(call),
                    Ok(StreamEvent::Usage(value)) => usage = value,
                    Ok(StreamEvent::Done { .. }) => {}
                    Err(error) => {
                        let latency_ms = started.elapsed().as_millis() as i64;
                        if has_usage(usage) {
                            self.record_usage(
                                &claimed.id,
                                &submission.conversation_id,
                                usage,
                                latency_ms,
                            )?;
                        }
                        return self.settle_failure(
                            &submission,
                            &claimed.id,
                            &format!("Provider error: {error}"),
                            "provider_error",
                        );
                    }
                }
            }
            let latency_ms = started.elapsed().as_millis() as i64;
            self.record_usage(&claimed.id, &submission.conversation_id, usage, latency_ms)?;
            self.session
                .heartbeat_task(&claimed.id, &self.config.owner, 60_000_000)?;

            if !calls.is_empty() {
                self.save_assistant(&submission, &prompt_hash, &thinking, &text, &calls)?;
                self.execute_calls(&submission, &claimed.id, &calls)?;
                turn += 1;
                self.session.checkpoint_task(
                    &claimed.id,
                    &self.config.owner,
                    json!({"turn": turn}),
                )?;
                continue;
            }
            if text.is_empty() {
                return self.settle_failure(
                    &submission,
                    &claimed.id,
                    "The model returned no answer.",
                    "empty_answer",
                );
            }

            let answer_entry_id =
                self.save_assistant(&submission, &prompt_hash, &thinking, &text, &[])?;
            let settled = self.session.settle(
                &submission.id,
                SubmissionStatus::Done,
                Some(&answer_entry_id),
            )?;
            self.session.finish_task(
                &claimed.id,
                &self.config.owner,
                StepOutput::Terminal {
                    outcome: TerminalOutcome::Done,
                    result: json!({"answer_entry_id": answer_entry_id}),
                },
            )?;
            return Ok(RunOutcome {
                submission: settled,
                task_id: claimed.id,
                answer_entry_id: Some(answer_entry_id),
                answer_text: Some(text),
                notice: None,
            });
        }
    }

    fn save_assistant(
        &self,
        submission: &Submission,
        prompt_hash: &str,
        thinking: &str,
        text: &str,
        calls: &[ToolCall],
    ) -> Result<String, AgentError> {
        let mut blocks = Vec::new();
        if !thinking.is_empty() {
            blocks.push(json!({"type": "thinking", "text": thinking}));
        }
        if !text.is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }
        for call in calls {
            blocks.push(json!({
                "type": "tool_call",
                "id": call.id,
                "name": call.name,
                "arguments": call.arguments,
            }));
        }
        let entry = NewEntry {
            kind: "assistant".to_owned(),
            content: Value::Array(blocks),
            source: None,
            prompt_hash: Some(prompt_hash.to_owned()),
        };
        let commit = self
            .session
            .save(Save::new(&submission.conversation_id).entry_with(entry))?;
        commit
            .entry_ids
            .first()
            .cloned()
            .ok_or_else(|| AgentError::Invalid("the assistant entry was not saved".to_owned()))
    }

    fn execute_calls(
        &self,
        submission: &Submission,
        task_id: &str,
        calls: &[ToolCall],
    ) -> Result<(), AgentError> {
        let mut blocks = Vec::new();
        for call in calls {
            let (content, is_error) = match self.config.tools.call(&call.name, &call.arguments) {
                Some(Ok(output)) => {
                    let (content, spill_failed) = self.spill_output(&call.id, &output.content);
                    (content, output.is_error || spill_failed)
                }
                Some(Err(error)) => (error.to_json().to_string(), true),
                None => {
                    let error = ToolError::invalid(
                        "unknown_tool",
                        format!("unknown tool: {}", call.name),
                    );
                    (error.to_json().to_string(), true)
                }
            };
            blocks.push(json!({
                "type": "tool_result",
                "call_id": call.id,
                "content": content,
                "is_error": is_error,
            }));
        }
        self.session.save(
            Save::new(&submission.conversation_id)
                .entry("tool_result", Value::Array(blocks)),
        )?;
        self.session
            .heartbeat_task(task_id, &self.config.owner, 60_000_000)?;
        Ok(())
    }

    fn spill_output(&self, call_id: &str, content: &str) -> (String, bool) {
        if hearth_model::estimate_tokens(content) <= 4000 {
            return (content.to_owned(), false);
        }
        let name = sanitize_call_id(call_id);
        if let Err(error) = fs::create_dir_all(&self.config.spill_dir) {
            return (
                format!("tool output was too large and the spill directory failed: {error}"),
                true,
            );
        }
        let path = self.config.spill_dir.join(format!("{name}.txt"));
        if let Err(error) = write_private(&path, content) {
            return (
                format!("tool output was too large and the spill write failed: {error}"),
                true,
            );
        }
        let head: String = content.chars().take(2000).collect();
        let tail: String = {
            let mut chars: Vec<char> = content.chars().rev().take(1000).collect();
            chars.reverse();
            chars.into_iter().collect()
        };
        (
            format!(
                "[tool output truncated: {} bytes, ~{} tokens; full output: spill/{name}.txt]\n{head}\n...\n{tail}",
                content.len(),
                hearth_model::estimate_tokens(content)
            ),
            false,
        )
    }

    fn record_usage(
        &self,
        task_id: &str,
        conversation_id: &str,
        usage: Usage,
        latency_ms: i64,
    ) -> Result<(), AgentError> {
        let cost_micros = self
            .pricing
            .price(&self.config.model)
            .map(|price| price.cost_micros(&usage))
            .unwrap_or(0);
        self.session.record_usage(NewUsage {
            task_id: Some(task_id.to_owned()),
            conversation_id: Some(conversation_id.to_owned()),
            provider: self.config.provider_name.clone(),
            model: self.config.model.clone(),
            kind: "chat".to_owned(),
            usage,
            cost_micros,
            latency_ms,
        })?;
        Ok(())
    }

    fn settle_failure(
        &self,
        submission: &Submission,
        task_id: &str,
        message: &str,
        reason: &str,
    ) -> Result<RunOutcome, AgentError> {
        let notice = short_notice(message);
        self.session.save(
            Save::new(&submission.conversation_id).entry("notice", json!({"text": notice})),
        )?;
        let settled = self.session.settle_with_error(
            &submission.id,
            SubmissionStatus::Unanswered,
            None,
            Some(json!({"reason": reason})),
        )?;
        self.session.finish_task(
            task_id,
            &self.config.owner,
            StepOutput::Terminal {
                outcome: TerminalOutcome::Failed,
                result: json!({"error": notice, "reason": reason}),
            },
        )?;
        Ok(RunOutcome {
            submission: settled,
            task_id: task_id.to_owned(),
            answer_entry_id: None,
            answer_text: None,
            notice: Some(notice),
        })
    }
}

struct Reservation<'a> {
    reservations: &'a Mutex<HashSet<String>>,
    conversation_id: String,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if let Ok(mut reserved) = self.reservations.lock() {
            reserved.remove(&self.conversation_id);
        }
    }
}

fn pending_calls(history: &[Entry]) -> Vec<ToolCall> {
    let mut results = HashSet::new();
    for entry in history {
        if entry.kind == "tool_result" {
            for block in parse_blocks(&entry.content) {
                if let ContentBlock::ToolResult { call_id, .. } = block {
                    results.insert(call_id);
                }
            }
        }
    }
    let mut pending = Vec::new();
    let mut seen = HashSet::new();
    for entry in history {
        if entry.kind == "assistant" {
            for block in parse_blocks(&entry.content) {
                if let ContentBlock::ToolCall(call) = block
                    && !results.contains(&call.id)
                    && seen.insert(call.id.clone())
                {
                    pending.push(call);
                }
            }
        }
    }
    pending
}

fn sanitize_call_id(call_id: &str) -> String {
    let name: String = call_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        "output".to_owned()
    } else {
        name
    }
}

fn write_private(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn has_usage(usage: Usage) -> bool {
    usage.input_tokens > 0
        || usage.cache_read_tokens > 0
        || usage.cache_write_tokens > 0
        || usage.output_tokens > 0
        || usage.reasoning_tokens > 0
}

fn short_notice(message: &str) -> String {
    let single = message.replace(['\n', '\r'], " ");
    let trimmed = single.trim();
    let text: String = trimmed.chars().take(200).collect();
    if text.is_empty() {
        "The run failed.".to_owned()
    } else {
        text
    }
}
