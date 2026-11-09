//! Helper tasks, their controls, and the model-facing task tool.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use hearth_config::Settings;
use hearth_core::{
    ContextBudget, CoreError, EffectIntent, EffectStatus, NewEntry, NewTask, Save, Session,
    StepOutput, SubmissionStatus, TaskStatus, TerminalOutcome,
};
use hearth_memory::{MemoryService, MemorySource};
use hearth_model::{Pricing, Provider, Speaker, ToolSpec};
use hearth_telegram::{SendOutcome, TelegramSender};
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};

use crate::agent::{Agent, AgentConfig};
use crate::events::EventSink;
use crate::tools::{MessageTarget, TaskTarget, ToolMediaRoots, build_tools};

/// Cancellation tokens shared by the task tool and the helper workers.
#[derive(Default)]
pub struct HelperControls {
    tokens: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl HelperControls {
    /// Create empty controls.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a fresh token for a running helper.
    pub fn register(&self, task_id: &str) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(task_id.to_owned(), Arc::clone(&token));
        }
        token
    }

    /// Cancel a running helper if one is registered.
    pub fn cancel(&self, task_id: &str) -> bool {
        let Ok(tokens) = self.tokens.lock() else {
            return false;
        };
        let Some(token) = tokens.get(task_id) else {
            return false;
        };
        token.store(true, Ordering::Relaxed);
        true
    }

    /// Drop a helper token after it settles.
    pub fn remove(&self, task_id: &str) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(task_id);
        }
    }
}

/// The model-facing task tool, bound to one parent conversation.
pub struct TaskTool {
    session: Session,
    conversation_id: String,
    chat_id: i64,
    thread_id: Option<i64>,
    depth: u8,
    controls: Arc<HelperControls>,
    max_depth: u8,
    max_concurrent: u8,
}

impl TaskTool {
    /// Build the task tool for one conversation and helper depth.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Session,
        conversation_id: impl Into<String>,
        chat_id: i64,
        thread_id: Option<i64>,
        depth: u8,
        controls: Arc<HelperControls>,
        max_depth: u8,
        max_concurrent: u8,
    ) -> Self {
        Self {
            session,
            conversation_id: conversation_id.into(),
            chat_id,
            thread_id,
            depth,
            controls,
            max_depth,
            max_concurrent,
        }
    }
}

impl Tool for TaskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "task".to_owned(),
            description: "Spawn a self-contained helper, or list, read, or kill helpers for this conversation.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["spawn", "list", "result", "kill"]},
                    "brief": {"type": "string", "description": "Objective, paths, constraints, and completion criteria"},
                    "expected_minutes": {"type": "integer", "minimum": 1},
                    "token_budget": {"type": "integer", "minimum": 1000},
                    "wall_seconds": {"type": "integer", "minimum": 1},
                    "tool_groups": {"type": "array", "items": {"type": "string"}},
                    "task_id": {"type": "string"}
                },
                "required": ["action"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "action is required"))?;
        match action {
            "spawn" => self.spawn(arguments),
            "list" => self.list(),
            "result" => self.result(arguments),
            "kill" => self.kill(arguments),
            other => Err(ToolError::invalid(
                "unknown_action",
                format!("unknown task action: {other}"),
            )),
        }
    }
}

impl TaskTool {
    fn spawn(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let brief = arguments
            .get("brief")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|brief| !brief.is_empty())
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "brief is required"))?;
        let child_depth = self.depth.saturating_add(1);
        if child_depth > self.max_depth {
            return Err(ToolError::invalid(
                "depth_limit",
                format!("helper depth limit is {}", self.max_depth),
            ));
        }
        let active = self
            .session
            .active_task_count("helper")
            .map_err(|error| ToolError::invalid("spawn_failed", error.to_string()))?;
        if active >= self.max_concurrent as usize {
            return Err(ToolError::invalid(
                "concurrency_limit",
                format!("at most {} helpers may run at once", self.max_concurrent),
            ));
        }
        let expected_minutes = arguments
            .get("expected_minutes")
            .and_then(Value::as_u64)
            .unwrap_or(10)
            .max(1);
        let token_budget = arguments.get("token_budget").and_then(Value::as_u64);
        let wall_seconds = arguments.get("wall_seconds").and_then(Value::as_i64);
        let tool_groups = arguments
            .get("tool_groups")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let now = self.session.now_micros();
        let key = format!("helper:{}:{}", self.conversation_id, now);
        let helper = self
            .session
            .create_conversation_with_parent("Helper", "helper", &key, &self.conversation_id)
            .map_err(|error| ToolError::invalid("spawn_failed", error.to_string()))?;
        let parent_task = self
            .session
            .active_generation_task(&self.conversation_id)
            .ok()
            .flatten();
        let mut new = NewTask::new(
            "helper",
            1,
            "run",
            json!({
                "brief": brief,
                "parent_conversation_id": self.conversation_id,
                "helper_conversation_id": helper.id,
                "chat_id": self.chat_id,
                "thread_id": self.thread_id,
                "depth": child_depth,
                "expected_seconds": expected_minutes.saturating_mul(60),
                "token_budget": token_budget,
                "wall_seconds": wall_seconds,
                "tool_groups": tool_groups,
            }),
        )
        .conversation(&helper.id);
        if let Some(parent_task) = parent_task {
            new = new.parent(&parent_task.id);
        }
        let task = self
            .session
            .create_task(new)
            .map_err(|error| ToolError::invalid("spawn_failed", error.to_string()))?;
        Ok(ToolOutput {
            content: json!({
                "task_id": task.id,
                "helper_conversation_id": helper.id,
                "status": "spawned",
            })
            .to_string(),
            is_error: false,
        })
    }

    fn list(&self) -> Result<ToolOutput, ToolError> {
        let tasks = self
            .session
            .tasks_by_kind("helper")
            .map_err(|error| ToolError::invalid("list_failed", error.to_string()))?;
        let items: Vec<Value> = tasks
            .into_iter()
            .filter(|task| {
                task.input
                    .get("parent_conversation_id")
                    .and_then(Value::as_str)
                    == Some(self.conversation_id.as_str())
            })
            .map(|task| {
                let brief = task
                    .input
                    .get("brief")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                json!({
                    "id": task.id,
                    "status": task.status.as_str(),
                    "phase": task.phase,
                    "created_at": task.created_at,
                    "brief": brief.chars().take(120).collect::<String>(),
                })
            })
            .collect();
        Ok(ToolOutput {
            content: Value::Array(items).to_string(),
            is_error: false,
        })
    }

    fn result(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let task_id = arguments
            .get("task_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "task_id is required"))?;
        let task = self.own_task(task_id)?;
        Ok(ToolOutput {
            content: json!({
                "task_id": task.id,
                "status": task.status.as_str(),
                "phase": task.phase,
                "result": task.result,
            })
            .to_string(),
            is_error: false,
        })
    }

    fn kill(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let task_id = arguments
            .get("task_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "task_id is required"))?;
        let task = self.own_task(task_id)?;
        if task.status.is_terminal() {
            return Ok(ToolOutput {
                content: json!({"task_id": task.id, "status": task.status.as_str()}).to_string(),
                is_error: false,
            });
        }
        self.controls.cancel(&task.id);
        let task = self
            .session
            .abort_task(&task.id, json!({"reason": "killed"}))
            .map_err(|error| ToolError::invalid("kill_failed", error.to_string()))?;
        Ok(ToolOutput {
            content: json!({"task_id": task.id, "status": task.status.as_str()}).to_string(),
            is_error: false,
        })
    }

    fn own_task(&self, task_id: &str) -> Result<hearth_core::Task, ToolError> {
        let task = self
            .session
            .task(task_id)
            .map_err(|error| ToolError::invalid("task_failed", error.to_string()))?
            .ok_or_else(|| ToolError::invalid("not_found", "task not found"))?;
        let owned = task
            .input
            .get("parent_conversation_id")
            .and_then(Value::as_str)
            == Some(self.conversation_id.as_str());
        if !owned || task.kind != "helper" {
            return Err(ToolError::invalid("not_found", "task not found"));
        }
        Ok(task)
    }
}

#[allow(dead_code)]
fn is_terminal(status: TaskStatus) -> bool {
    status.is_terminal()
}

/// Everything a helper worker needs.
pub struct HelperConfig {
    pub settings: Arc<Settings>,
    pub session: Session,
    pub provider: Arc<dyn Provider>,
    pub sender: TelegramSender,
    pub speaker: Option<Arc<dyn Speaker>>,
    pub memory: Arc<MemoryService>,
    pub soul: String,
    pub workspace: PathBuf,
    pub spill: PathBuf,
    pub outbound: PathBuf,
    pub media: ToolMediaRoots,
    pub model: String,
    pub provider_name: String,
    pub voice_tags: bool,
    pub max_turns: u32,
    pub budget: ContextBudget,
    pub pricing: Pricing,
    pub events: Arc<dyn EventSink>,
    pub owner: String,
}

/// A pool of helper workers sharing one configuration and cancellation map.
#[derive(Clone)]
pub struct HelperPool {
    config: Arc<HelperConfig>,
    controls: Arc<HelperControls>,
}

impl HelperPool {
    /// Build a helper pool.
    pub fn new(config: Arc<HelperConfig>, controls: Arc<HelperControls>) -> Self {
        Self { config, controls }
    }

    /// Claim and run at most one helper task.
    pub fn run_once(&self) -> Result<bool, CoreError> {
        let Some(task) =
            self.config
                .session
                .claim_task_for_kind(&self.config.owner, 3_600_000_000, "helper")?
        else {
            return Ok(false);
        };
        self.run_helper(task);
        Ok(true)
    }

    /// Run until shutdown.
    pub fn run(&self, shutdown: Arc<AtomicBool>) {
        while !shutdown.load(Ordering::Relaxed) {
            match self.run_once() {
                Ok(true) => {}
                Ok(false) => thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    tracing::warn!(%error, "helper pool iteration failed");
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    /// Start one worker thread per configured concurrent helper.
    pub fn spawn_workers(&self, shutdown: Arc<AtomicBool>) -> Vec<thread::JoinHandle<()>> {
        let count = self.config.settings.tasks.max_concurrent.max(1);
        (0..count)
            .map(|_| {
                let pool = self.clone();
                let stop = Arc::clone(&shutdown);
                thread::spawn(move || pool.run(stop))
            })
            .collect()
    }

    fn run_helper(&self, task: hearth_core::ClaimedTask) {
        let task_id = task.id.clone();
        let Some(helper_id) = task.conversation_id.clone() else {
            tracing::warn!(task = %task_id, "helper task has no conversation");
            return;
        };
        let Some(brief) = task
            .input
            .get("brief")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            tracing::warn!(task = %task_id, "helper task has no brief");
            return;
        };
        let parent_id = task
            .input
            .get("parent_conversation_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let chat_id = task.input.get("chat_id").and_then(Value::as_i64);
        let thread_id = task.input.get("thread_id").and_then(Value::as_i64);
        let depth = task.input.get("depth").and_then(Value::as_u64).unwrap_or(1) as u8;
        let expected = task
            .input
            .get("expected_seconds")
            .and_then(Value::as_i64)
            .unwrap_or(600);
        let token_budget = task
            .input
            .get("token_budget")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.budget.context_window);
        let wall_seconds = task
            .input
            .get("wall_seconds")
            .and_then(Value::as_i64)
            .unwrap_or(3600)
            .max(1);
        let helper_budget = if token_budget < self.config.budget.context_window {
            ContextBudget::new(
                token_budget,
                self.config.budget.max_output_tokens,
                0.65,
                0.85,
                self.config.budget.keep_recent,
            )
        } else {
            self.config.budget
        };
        let cancel = self.controls.register(&task_id);

        let submission = match self.config.session.submit(
            &helper_id,
            &format!("helper:{task_id}"),
            "input",
            json!({"text": brief}),
        ) {
            Ok(result) => result.submission,
            Err(error) => {
                tracing::warn!(task = %task_id, %error, "helper submission failed");
                let _ = self.config.session.finish_task(
                    &task_id,
                    &self.config.owner,
                    StepOutput::Failed {
                        error: json!({"error": error.to_string()}),
                        retryable: false,
                    },
                );
                self.controls.remove(&task_id);
                return;
            }
        };

        let Some(chat_id) = chat_id else {
            tracing::warn!(task = %task_id, "helper task has no chat id");
            let _ = self.config.session.finish_task(
                &task_id,
                &self.config.owner,
                StepOutput::Failed {
                    error: json!({"error": "helper has no notification chat"}),
                    retryable: false,
                },
            );
            self.controls.remove(&task_id);
            return;
        };

        let target = MessageTarget {
            session: self.config.session.clone(),
            conversation_id: helper_id.clone(),
            sender: self.config.sender.clone(),
            chat_id,
            thread_id,
            outbound: self.config.outbound.clone(),
            speaker: self.config.speaker.clone(),
        };
        let memory_source = self
            .config
            .session
            .latest_user_entry_id(&helper_id)
            .ok()
            .flatten()
            .map(MemorySource::user);
        let task_target = TaskTarget {
            depth,
            controls: Arc::clone(&self.controls),
            max_depth: self.config.settings.tasks.max_depth,
            max_concurrent: self.config.settings.tasks.max_concurrent,
        };
        let built = match build_tools(
            &self.config.settings,
            &self.config.workspace,
            &self.config.spill,
            Some(target),
            Some(Arc::clone(&self.config.memory)),
            memory_source,
            None,
            Some(task_target),
            &self.config.media,
        ) {
            Ok(built) => built,
            Err(error) => {
                tracing::warn!(task = %task_id, %error, "helper tools failed");
                let _ = self.config.session.finish_task(
                    &task_id,
                    &self.config.owner,
                    StepOutput::Failed {
                        error: json!({"error": error.to_string()}),
                        retryable: false,
                    },
                );
                self.controls.remove(&task_id);
                return;
            }
        };
        if let Some(groups) = task.input.get("tool_groups").and_then(Value::as_array)
            && let Ok(mut selection) = built.selection.lock()
        {
            for group in groups.iter().filter_map(Value::as_str) {
                selection.insert(group.to_owned());
            }
        }

        let agent = Agent::new(
            self.config.session.clone(),
            Arc::clone(&self.config.provider),
            self.config.pricing.clone(),
            AgentConfig {
                soul: self.config.soul.clone(),
                model: self.config.model.clone(),
                provider_name: self.config.provider_name.clone(),
                owner: format!("{}:{}", self.config.owner, task_id),
                max_turns: self.config.max_turns,
                tools: built.core,
                spill_dir: self.config.spill.clone(),
                voice_tags: self.config.voice_tags,
            },
        )
        .with_memory(Arc::clone(&self.config.memory))
        .with_groups(built.groups, built.selection)
        .with_budget(helper_budget)
        .without_extraction();

        let (wall_tx, wall_rx) = mpsc::channel::<()>();
        let wall_cancel = Arc::clone(&cancel);
        let wall_handle = thread::spawn(move || {
            if wall_rx
                .recv_timeout(Duration::from_secs(wall_seconds as u64))
                .is_err()
            {
                wall_cancel.store(true, Ordering::Relaxed);
            }
        });

        let (eta_tx, eta_rx) = mpsc::channel::<()>();
        let eta_session = self.config.session.clone();
        let eta_sender = self.config.sender.clone();
        let eta_parent = parent_id.clone();
        let eta_task = task_id.clone();
        let eta_handle = thread::spawn(move || {
            if eta_rx
                .recv_timeout(Duration::from_secs(expected.max(0) as u64))
                .is_err()
                && !eta_parent.is_empty()
            {
                let text = format!("Helper {eta_task} is taking longer than expected.");
                let _ = send_notice(
                    &eta_session,
                    &eta_sender,
                    &eta_parent,
                    chat_id,
                    thread_id,
                    &format!("helper.eta:{eta_task}"),
                    &text,
                );
            }
        });

        let outcome = agent.run_submission_with_events(
            &submission.id,
            Arc::clone(&cancel),
            Arc::clone(&self.config.events),
        );
        let _ = eta_tx.send(());
        let _ = eta_handle.join();
        let _ = wall_tx.send(());
        let _ = wall_handle.join();
        self.controls.remove(&task_id);

        if let Ok(Some(current)) = self.config.session.task(&task_id)
            && current.status.is_terminal()
        {
            return;
        }

        let (report, succeeded) = match &outcome {
            Ok(outcome) => {
                let text = outcome
                    .answer_text
                    .clone()
                    .or_else(|| outcome.notice.clone())
                    .unwrap_or_else(|| "Helper finished without a report.".to_owned());
                (text, outcome.submission.status == SubmissionStatus::Done)
            }
            Err(error) => (format!("Helper failed: {error}"), false),
        };

        let report_path = self.config.spill.join(format!("helper-{task_id}.txt"));
        let wrote_report = write_report(&report_path, &report).is_ok();
        if !wrote_report {
            tracing::warn!(task = %task_id, "helper report file could not be written");
        }
        if !parent_id.is_empty() {
            let entry = NewEntry {
                kind: "assistant".to_owned(),
                content: json!({"text": report}),
                source: Some(format!("helper:{task_id}")),
                prompt_hash: None,
            };
            if let Err(error) = self
                .config
                .session
                .save(Save::new(&parent_id).entry_with(entry))
            {
                tracing::warn!(task = %task_id, %error, "helper report entry failed");
            }
            let text = format!(
                "Helper finished.\n\n{report}\n\nReport: {}",
                report_path.display()
            );
            let _ = send_notice(
                &self.config.session,
                &self.config.sender,
                &parent_id,
                chat_id,
                thread_id,
                &format!("helper.notice:{task_id}"),
                &text,
            );
        }

        let result = json!({
            "report_path": report_path.display().to_string(),
            "answer_entry_id": match &outcome {
                Ok(outcome) => outcome.answer_entry_id.clone(),
                Err(_) => None,
            },
            "status": if succeeded { "done" } else { "failed" },
        });
        let step = StepOutput::Terminal {
            outcome: if succeeded {
                TerminalOutcome::Done
            } else {
                TerminalOutcome::Failed
            },
            result,
        };
        if let Err(error) = self
            .config
            .session
            .finish_task(&task_id, &self.config.owner, step)
        {
            tracing::warn!(task = %task_id, %error, "helper task finish failed");
        }
    }
}

fn write_report(path: &PathBuf, report: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(report.as_bytes())?;
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn send_notice(
    session: &Session,
    sender: &TelegramSender,
    parent: &str,
    chat_id: i64,
    thread_id: Option<i64>,
    key: &str,
    text: &str,
) -> Result<(), CoreError> {
    let existing = session.effect_by_key("telegram.send", key)?;
    let effect_id = match existing {
        Some(effect) => match effect.status {
            EffectStatus::Done | EffectStatus::Unknown | EffectStatus::Failed => return Ok(()),
            EffectStatus::Started => {
                session.mark_effect_unknown(
                    &effect.id,
                    json!({"message": "helper notification was interrupted"}),
                )?;
                return Ok(());
            }
            EffectStatus::Intent => effect.id,
        },
        None => {
            let payload = json!({
                "chat_id": chat_id,
                "thread_id": thread_id,
                "markdown": text,
                "helper": true,
            });
            let commit = session.save(Save::new(parent).effect(EffectIntent::new(
                "telegram.send",
                key,
                payload,
            )))?;
            commit.effect_ids.first().cloned().ok_or_else(|| {
                CoreError::Invalid("helper notification effect was not saved".into())
            })?
        }
    };
    session.begin_effect(&effect_id)?;
    match sender.send_reply(chat_id, thread_id, text) {
        Ok(SendOutcome::Rich) | Ok(SendOutcome::Fallback { .. }) => {
            session.finish_effect(&effect_id, json!({"delivered": true, "helper": true}))?;
            Ok(())
        }
        Ok(SendOutcome::Uncertain { message }) => {
            session.mark_effect_unknown(&effect_id, json!({"message": message}))?;
            Ok(())
        }
        Err(error) => {
            session.mark_effect_failed(&effect_id, json!({"error": error.to_string()}))?;
            Err(CoreError::Invalid(error.to_string()))
        }
    }
}
