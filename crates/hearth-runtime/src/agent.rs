//! The agent loop: one submission, one durable generation task, one answer.

use std::sync::Arc;
use std::time::Instant;

use hearth_core::{
    CoreError, NewEntry, NewTask, NewUsage, PromptInput, Save, Session, StepOutput, Submission,
    SubmissionStatus, TerminalOutcome,
};
use hearth_model::{Pricing, Provider, StreamEvent, Usage};
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
        }
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
            tools: Vec::new(),
            runtime_note,
        })?;
        let prompt_text = serde_json::to_string(&prompt.document)
            .map_err(|error| AgentError::Invalid(error.to_string()))?;
        let prompt_hash = self.session.save_prompt_blob(&prompt_text)?;
        self.session.checkpoint_task(
            &claimed.id,
            &self.config.owner,
            json!({"turn": 0, "prompt_hash": prompt_hash}),
        )?;

        let started = Instant::now();
        let stream = match self.provider.stream(prompt.request) {
            Ok(stream) => stream,
            Err(error) => {
                return self.settle_failure(
                    &submission,
                    &claimed.id,
                    &format!("Provider error: {error}"),
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
                    );
                }
            }
        }
        let latency_ms = started.elapsed().as_millis() as i64;
        self.record_usage(&claimed.id, &submission.conversation_id, usage, latency_ms)?;
        self.session
            .heartbeat_task(&claimed.id, &self.config.owner, 60_000_000)?;

        if !calls.is_empty() {
            return self.settle_failure(
                &submission,
                &claimed.id,
                "The model requested a tool, but no tools are available yet.",
            );
        }
        if text.is_empty() && thinking.is_empty() {
            return self.settle_failure(
                &submission,
                &claimed.id,
                "The model returned an empty response.",
            );
        }

        let mut blocks = Vec::new();
        if !thinking.is_empty() {
            blocks.push(json!({"type": "thinking", "text": thinking}));
        }
        let answer_text = if text.is_empty() {
            None
        } else {
            Some(text.clone())
        };
        if !text.is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }
        let entry = NewEntry {
            kind: "assistant".to_owned(),
            content: Value::Array(blocks),
            source: None,
            prompt_hash: Some(prompt_hash),
        };
        let commit = self
            .session
            .save(Save::new(&submission.conversation_id).entry_with(entry))?;
        let answer_entry_id = commit
            .entry_ids
            .first()
            .cloned()
            .ok_or_else(|| AgentError::Invalid("the answer entry was not saved".to_owned()))?;
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
        Ok(RunOutcome {
            submission: settled,
            task_id: claimed.id,
            answer_entry_id: Some(answer_entry_id),
            answer_text,
            notice: None,
        })
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
    ) -> Result<RunOutcome, AgentError> {
        let notice = short_notice(message);
        self.session.save(
            Save::new(&submission.conversation_id).entry("notice", json!({"text": notice})),
        )?;
        let settled = self
            .session
            .settle(&submission.id, SubmissionStatus::Unanswered, None)?;
        self.session.finish_task(
            task_id,
            &self.config.owner,
            StepOutput::Terminal {
                outcome: TerminalOutcome::Failed,
                result: json!({"error": notice}),
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
