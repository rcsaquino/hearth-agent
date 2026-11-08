//! Helper tasks, their controls, and the model-facing task tool.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use hearth_core::{NewTask, Session, TaskStatus};
use hearth_model::ToolSpec;
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};

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
