use std::sync::Arc;

use hearth_core::{Harness, TaskStatus};
use hearth_runtime::{HelperControls, TaskTool};
use hearth_tools::Tool;
use serde_json::{Value, json};
use tempfile::TempDir;

fn setup() -> (TempDir, Harness, hearth_core::Conversation, TaskTool) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let conversation = harness
        .session()
        .create_conversation("Parent", "telegram", "42:0")
        .expect("parent");
    let tool = TaskTool::new(
        harness.session().clone(),
        &conversation.id,
        42,
        None,
        0,
        Arc::new(HelperControls::new()),
        2,
        4,
    );
    (home, harness, conversation, tool)
}

fn content(output: &hearth_tools::ToolOutput) -> Value {
    serde_json::from_str(&output.content).expect("tool json")
}

#[test]
fn spawn_creates_a_child_conversation_and_helper_task() {
    let (_home, harness, parent, tool) = setup();
    let out = tool
        .call(&json!({
            "action": "spawn",
            "brief": "Read the logs and summarize them.",
            "expected_minutes": 5,
            "tool_groups": ["web"]
        }))
        .expect("spawn");
    assert!(!out.is_error);
    let value = content(&out);
    let task_id = value["task_id"].as_str().expect("task id");
    let session = harness.session();
    let task = session.task(task_id).expect("read").expect("task");
    assert_eq!(task.kind, "helper");
    assert_eq!(task.status, TaskStatus::Pending);
    assert_eq!(task.input["parent_conversation_id"], parent.id);
    assert_eq!(task.input["chat_id"], 42);
    assert_eq!(task.input["depth"], 1);
    let helper = session
        .conversation(task.conversation_id.as_deref().expect("conversation"))
        .expect("lookup")
        .expect("helper conversation");
    assert_eq!(helper.parent_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(helper.transport, "helper");

    let listed = tool.call(&json!({"action": "list"})).expect("list");
    let items = content(&listed);
    assert!(items.as_array().expect("array").iter().any(|item| item["id"] == task_id));

    let result = tool
        .call(&json!({"action": "result", "task_id": task_id}))
        .expect("result");
    assert_eq!(content(&result)["status"], "pending");
}

#[test]
fn kill_aborts_a_pending_helper() {
    let (_home, harness, _parent, tool) = setup();
    let out = tool
        .call(&json!({"action": "spawn", "brief": "A job"}))
        .expect("spawn");
    let task_id = content(&out)["task_id"].as_str().expect("task id").to_owned();
    let killed = tool
        .call(&json!({"action": "kill", "task_id": task_id}))
        .expect("kill");
    assert!(!killed.is_error);
    let task = harness
        .session()
        .task(&task_id)
        .expect("read")
        .expect("task");
    assert_eq!(task.status, TaskStatus::Aborted);
}

#[test]
fn depth_beyond_the_limit_is_rejected() {
    let (_home, harness, parent, _tool) = setup();
    let deep = TaskTool::new(
        harness.session().clone(),
        &parent.id,
        42,
        None,
        2,
        Arc::new(HelperControls::new()),
        2,
        4,
    );
    assert!(deep
        .call(&json!({"action": "spawn", "brief": "Too deep"}))
        .is_err());
}

#[test]
fn concurrency_limit_is_enforced() {
    let _home = TempDir::new().expect("home");
    let harness = Harness::open(_home.path()).expect("harness");
    let parent = harness
        .session()
        .create_conversation("Parent", "telegram", "42:0")
        .expect("parent");
    let tool = TaskTool::new(
        harness.session().clone(),
        &parent.id,
        42,
        None,
        0,
        Arc::new(HelperControls::new()),
        2,
        2,
    );
    tool.call(&json!({"action": "spawn", "brief": "One"}))
        .expect("first");
    tool.call(&json!({"action": "spawn", "brief": "Two"}))
        .expect("second");
    assert!(tool
        .call(&json!({"action": "spawn", "brief": "Three"}))
        .is_err());
}
