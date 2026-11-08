//! The background task scheduler, reminder handler, and schedule tool.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use hearth_core::{
    ClaimedTask, CoreError, EffectIntent, EffectStatus, NewTask, Save, Session, StepOutput,
    TerminalOutcome,
};
use hearth_model::ToolSpec;
use hearth_telegram::{SendOutcome, TelegramSender};
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A handler failure that the scheduler turns into a task transition.
#[derive(Debug, Error)]
pub enum TaskHandlerError {
    #[error("{0}")]
    Retryable(String),
    #[error("{0}")]
    Permanent(String),
}

impl TaskHandlerError {
    fn from_core(error: CoreError) -> Self {
        Self::Retryable(error.to_string())
    }
}

/// One step handler for a task kind.
pub trait TaskHandler: Send + Sync {
    fn step(&self, task: &ClaimedTask) -> Result<StepOutput, TaskHandlerError>;
}

/// Handlers keyed by the task kind they own.
#[derive(Default, Clone)]
pub struct TaskRegistry {
    handlers: BTreeMap<String, Arc<dyn TaskHandler>>,
}

impl TaskRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one handler.
    pub fn register(mut self, kind: impl Into<String>, handler: Arc<dyn TaskHandler>) -> Self {
        self.handlers.insert(kind.into(), handler);
        self
    }

    /// The kinds this registry can run.
    pub fn kinds(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    /// One handler by kind.
    pub fn handler(&self, kind: &str) -> Option<Arc<dyn TaskHandler>> {
        self.handlers.get(kind).cloned()
    }
}

/// Claims runnable tasks for the registered handlers and saves each transition.
pub struct Scheduler {
    session: Session,
    registry: Arc<TaskRegistry>,
    owner: String,
    lease_micros: i64,
}

impl Scheduler {
    /// Build a scheduler with a 60 second lease.
    pub fn new(session: Session, registry: Arc<TaskRegistry>, owner: impl Into<String>) -> Self {
        Self {
            session,
            registry,
            owner: owner.into(),
            lease_micros: 60_000_000,
        }
    }

    /// Run until shutdown.
    pub fn run(&self, shutdown: Arc<AtomicBool>) {
        while !shutdown.load(Ordering::Relaxed) {
            if let Err(error) = self.tick() {
                tracing::warn!(%error, "scheduler iteration failed");
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn tick(&self) -> Result<(), CoreError> {
        for kind in self.registry.kinds() {
            let Some(task) =
                self.session
                    .claim_task_for_kind(&self.owner, self.lease_micros, &kind)?
            else {
                continue;
            };
            if let Err(error) =
                self.session
                    .heartbeat_task(&task.id, &self.owner, self.lease_micros)
            {
                tracing::warn!(task = %task.id, %error, "scheduler heartbeat failed");
            }
            let output = match self.registry.handler(&kind) {
                Some(handler) => match handler.step(&task) {
                    Ok(output) => output,
                    Err(TaskHandlerError::Retryable(message)) => StepOutput::Failed {
                        error: json!({"error": message}),
                        retryable: true,
                    },
                    Err(TaskHandlerError::Permanent(message)) => StepOutput::Failed {
                        error: json!({"error": message}),
                        retryable: false,
                    },
                },
                None => StepOutput::Failed {
                    error: json!({"error": format!("no handler for task kind {kind}")}),
                    retryable: false,
                },
            };
            let _ = self
                .session
                .heartbeat_task(&task.id, &self.owner, self.lease_micros);
            if let Err(error) = self.session.finish_task(&task.id, &self.owner, output) {
                tracing::warn!(task = %task.id, %error, "scheduler transition failed");
            } else {
                tracing::info!(task = %task.id, kind = %kind, "scheduler task step");
            }
            return Ok(());
        }
        Ok(())
    }
}

/// Sends reminder tasks and reschedules recurring ones.
pub struct ReminderHandler {
    session: Session,
    sender: TelegramSender,
}

impl ReminderHandler {
    /// Build the reminder handler.
    pub fn new(session: Session, sender: TelegramSender) -> Self {
        Self { session, sender }
    }

    fn deliver(&self, task: &ClaimedTask) -> Result<EffectStatus, TaskHandlerError> {
        let conversation_id = task
            .conversation_id
            .clone()
            .ok_or_else(|| TaskHandlerError::Permanent("reminder has no conversation".into()))?;
        let chat_id = task
            .input
            .get("chat_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| TaskHandlerError::Permanent("reminder has no chat id".into()))?;
        let thread_id = task.input.get("thread_id").and_then(Value::as_i64);
        let message = task
            .input
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .ok_or_else(|| TaskHandlerError::Permanent("reminder has no message".into()))?;
        let scheduled_wake = schedule_wake(task);
        let key = format!("reminder:{}:{}", task.id, scheduled_wake);
        let payload = json!({
            "chat_id": chat_id,
            "thread_id": thread_id,
            "markdown": message,
            "reminder": true,
            "task_id": task.id,
            "scheduled_wake": scheduled_wake,
        });

        let existing = self
            .session
            .effect_by_key("telegram.send", &key)
            .map_err(TaskHandlerError::from_core)?;
        let effect_id = match existing {
            Some(effect) => match effect.status {
                EffectStatus::Done => return Ok(EffectStatus::Done),
                EffectStatus::Unknown => return Ok(EffectStatus::Unknown),
                EffectStatus::Started => {
                    self.session
                        .mark_effect_unknown(
                            &effect.id,
                            json!({"message": "reminder delivery was interrupted"}),
                        )
                        .map_err(TaskHandlerError::from_core)?;
                    return Ok(EffectStatus::Unknown);
                }
                EffectStatus::Failed => {
                    return Err(TaskHandlerError::Permanent(
                        effect
                            .result
                            .and_then(|result| {
                                result
                                    .get("error")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                            })
                            .unwrap_or_else(|| "reminder delivery failed".to_owned()),
                    ));
                }
                EffectStatus::Intent => effect.id,
            },
            None => {
                let commit = self
                    .session
                    .save(Save::new(&conversation_id).effect(EffectIntent::new(
                        "telegram.send",
                        key,
                        payload,
                    )))
                    .map_err(TaskHandlerError::from_core)?;
                commit.effect_ids.first().cloned().ok_or_else(|| {
                    TaskHandlerError::Permanent("reminder effect was not saved".into())
                })?
            }
        };
        self.session
            .begin_effect(&effect_id)
            .map_err(TaskHandlerError::from_core)?;
        match self.sender.send_reply(chat_id, thread_id, message) {
            Ok(SendOutcome::Rich) | Ok(SendOutcome::Fallback { .. }) => {
                self.session
                    .finish_effect(&effect_id, json!({"delivered": true, "reminder": true}))
                    .map_err(TaskHandlerError::from_core)?;
                Ok(EffectStatus::Done)
            }
            Ok(SendOutcome::Uncertain { message }) => {
                self.session
                    .mark_effect_unknown(&effect_id, json!({"message": message}))
                    .map_err(TaskHandlerError::from_core)?;
                Err(TaskHandlerError::Retryable(message))
            }
            Err(error) => {
                let message = error.to_string();
                self.session
                    .mark_effect_failed(&effect_id, json!({"error": message}))
                    .map_err(TaskHandlerError::from_core)?;
                Err(TaskHandlerError::Permanent(message))
            }
        }
    }

    fn advance(
        &self,
        task: &ClaimedTask,
        status: EffectStatus,
    ) -> Result<StepOutput, TaskHandlerError> {
        let repeat = task
            .input
            .get("repeat_seconds")
            .and_then(Value::as_i64)
            .filter(|seconds| *seconds > 0);
        let delivery = match status {
            EffectStatus::Done => "sent",
            EffectStatus::Unknown => "unknown",
            _ => "unknown",
        };
        if let Some(seconds) = repeat {
            let now = self.session.now_micros();
            let next = now.saturating_add(seconds.saturating_mul(1_000_000));
            return Ok(StepOutput::Sleep {
                until_micros: next,
                phase: "remind".to_owned(),
                checkpoint: json!({
                    "scheduled_wake": next,
                    "last_delivery": delivery,
                }),
            });
        }
        Ok(StepOutput::Terminal {
            outcome: TerminalOutcome::Done,
            result: json!({
                "delivery": delivery,
                "scheduled_wake": schedule_wake(task),
            }),
        })
    }
}

impl TaskHandler for ReminderHandler {
    fn step(&self, task: &ClaimedTask) -> Result<StepOutput, TaskHandlerError> {
        let status = self.deliver(task)?;
        self.advance(task, status)
    }
}

fn schedule_wake(task: &ClaimedTask) -> i64 {
    task.checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.get("scheduled_wake"))
        .and_then(Value::as_i64)
        .or_else(|| task.input.get("first_wake").and_then(Value::as_i64))
        .unwrap_or(0)
}

/// The model-facing schedule tool, bound to one Telegram conversation.
pub struct ScheduleTool {
    session: Session,
    conversation_id: String,
    chat_id: i64,
    thread_id: Option<i64>,
}

impl ScheduleTool {
    /// Build the tool for one conversation.
    pub fn new(
        session: Session,
        conversation_id: impl Into<String>,
        chat_id: i64,
        thread_id: Option<i64>,
    ) -> Self {
        Self {
            session,
            conversation_id: conversation_id.into(),
            chat_id,
            thread_id,
        }
    }
}

impl Tool for ScheduleTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "schedule".to_owned(),
            description: "Create a reminder that arrives later in this chat. Use an RFC3339 time or a delay in seconds; repeat_seconds makes it recurring.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string", "description": "What to send when the reminder fires"},
                    "at": {"type": "string", "description": "Absolute RFC3339 time"},
                    "delay_seconds": {"type": "integer", "minimum": 0, "description": "Seconds from now"},
                    "repeat_seconds": {"type": "integer", "minimum": 1, "description": "Optional recurring interval"}
                },
                "required": ["message"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let message = arguments
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "message is required"))?;
        let at = arguments.get("at").and_then(Value::as_str);
        let delay = arguments.get("delay_seconds").and_then(Value::as_i64);
        let now = self.session.now_micros();
        let wake_at = match (at, delay) {
            (Some(at), None) => OffsetDateTime::parse(at, &Rfc3339)
                .map_err(|error| {
                    ToolError::invalid("invalid_time", format!("invalid RFC3339 time: {error}"))
                })?
                .unix_timestamp_nanos()
                .div_euclid(1_000) as i64,
            (None, Some(seconds)) if seconds >= 0 => {
                now.saturating_add(seconds.saturating_mul(1_000_000))
            }
            _ => {
                return Err(ToolError::invalid(
                    "invalid_arguments",
                    "give exactly one of at or delay_seconds",
                ));
            }
        };
        let repeat = arguments
            .get("repeat_seconds")
            .and_then(Value::as_i64)
            .filter(|seconds| *seconds > 0);
        let repeat_value = repeat.map(Value::from).unwrap_or(Value::Null);
        let task = NewTask::new(
            "reminder",
            1,
            "remind",
            json!({
                "chat_id": self.chat_id,
                "thread_id": self.thread_id,
                "message": message,
                "repeat_seconds": repeat_value,
                "first_wake": wake_at,
            }),
        )
        .conversation(&self.conversation_id);
        let task = self
            .session
            .create_sleeping_task(task, wake_at)
            .map_err(|error| ToolError::invalid("schedule_failed", error.to_string()))?;
        Ok(ToolOutput {
            content: json!({"task_id": task.id, "wake_at": wake_at}).to_string(),
            is_error: false,
        })
    }
}
