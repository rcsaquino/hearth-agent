use hearth_core::{Harness, NewTask, TaskStatus};
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    (home, harness)
}

#[test]
fn a_child_conversation_links_to_its_parent() {
    let (_home, harness) = harness();
    let session = harness.session();
    let parent = session
        .create_conversation("Parent", "telegram", "42:0")
        .expect("parent");
    let child = session
        .create_conversation_with_parent("Helper", "helper", "helper:1", &parent.id)
        .expect("child");
    assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.parent_entry_id, None);
    assert_eq!(child.transport, "helper");
    assert_eq!(child.title, "Helper");
}

#[test]
fn aborting_a_task_settles_it_once() {
    let (_home, harness) = harness();
    let session = harness.session();
    let task = session
        .create_task(NewTask::new("helper", 1, "run", json!({"brief": "x"})))
        .expect("task");
    let aborted = session
        .abort_task(&task.id, json!({"reason": "killed"}))
        .expect("abort");
    assert_eq!(aborted.status, TaskStatus::Aborted);
    assert_eq!(aborted.result.as_ref().expect("result")["reason"], "killed");
    assert!(aborted.terminal_at.is_some());
    assert_eq!(aborted.lease_owner, None);
    let again = session
        .abort_task(&task.id, json!({"reason": "twice"}))
        .expect("idempotent abort");
    assert_eq!(again.status, TaskStatus::Aborted);
    assert_eq!(again.result.as_ref().expect("result")["reason"], "killed");
}

#[test]
fn active_task_count_counts_pending_and_running() {
    let (_home, harness) = harness();
    let session = harness.session();
    let first = session
        .create_task(NewTask::new("helper", 1, "run", json!({})))
        .expect("first");
    let second = session
        .create_task(NewTask::new("helper", 1, "run", json!({})))
        .expect("second");
    assert_eq!(session.active_task_count("helper").expect("count"), 2);
    let claimed = session
        .claim_task_for_kind("owner", 60_000_000, "helper")
        .expect("claim")
        .expect("task");
    assert!(claimed.id == first.id || claimed.id == second.id);
    session
        .abort_task(&claimed.id, json!({"reason": "killed"}))
        .expect("abort");
    assert_eq!(session.active_task_count("helper").expect("count"), 1);
}

#[test]
fn helper_tasks_can_be_listed_newest_first() {
    let (_home, harness) = harness();
    let session = harness.session();
    let parent = session
        .create_conversation("Parent", "telegram", "42:0")
        .expect("parent");
    let first = session
        .create_task(
            NewTask::new(
                "helper",
                1,
                "run",
                json!({"parent_conversation_id": parent.id}),
            )
            .conversation(&parent.id),
        )
        .expect("first");
    let second = session
        .create_task(
            NewTask::new(
                "helper",
                1,
                "run",
                json!({"parent_conversation_id": parent.id}),
            )
            .conversation(&parent.id),
        )
        .expect("second");
    let tasks = session.tasks_by_kind("helper").expect("list");
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, second.id);
    assert_eq!(tasks[1].id, first.id);
}

#[test]
fn helper_retention_archives_runs_beyond_the_newest_limit() {
    let (_home, harness) = harness();
    let session = harness.session();
    let parent = session
        .create_conversation("Parent", "telegram", "42:0")
        .expect("parent");
    let now = session.now_micros();
    for index in 0..30 {
        let helper = session
            .create_conversation_with_parent(
                "Helper",
                "helper",
                format!("helper:{index}:{now}"),
                &parent.id,
            )
            .expect("helper conversation");
        session
            .create_task(
                NewTask::new(
                    "helper",
                    1,
                    "run",
                    json!({"parent_conversation_id": parent.id, "brief": format!("job {index}")}),
                )
                .conversation(&helper.id),
            )
            .expect("helper task");
    }
    let future = now.saturating_add(100_i64 * 86_400_000_000);
    let pruned = session
        .prune_helpers(30, 25, future)
        .expect("prune helpers");
    assert_eq!(pruned, 5);
    let tasks = session.tasks_by_kind("helper").expect("tasks");
    assert_eq!(tasks.len(), 30);
    let archived = tasks
        .iter()
        .filter(|task| {
            task.conversation_id
                .as_deref()
                .and_then(|id| session.conversation(id).ok().flatten())
                .map(|conversation| conversation.archived)
                .unwrap_or(false)
        })
        .count();
    assert_eq!(archived, 5);
    let parent = session
        .conversation(&parent.id)
        .expect("parent lookup")
        .expect("parent exists");
    assert!(!parent.archived);
}
