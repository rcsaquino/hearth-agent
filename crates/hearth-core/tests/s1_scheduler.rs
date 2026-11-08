use hearth_core::{Harness, NewTask, TaskStatus};
use serde_json::json;
use tempfile::TempDir;

fn home() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

#[test]
fn a_sleeping_task_waits_for_its_wake() {
    let (_home, harness) = home();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let now = session.now_micros();
    let later = session
        .create_sleeping_task(
            NewTask::new("reminder", 1, "remind", json!({"message": "later"}))
                .conversation(&conversation.id),
            now + 3_600_000_000,
        )
        .expect("sleeping task");
    assert_eq!(later.status, TaskStatus::Asleep);
    assert_eq!(later.next_wake_at, Some(now + 3_600_000_000));
    assert!(
        session
            .claim_task_for_kind("scheduler", 60_000_000, "reminder")
            .expect("claim")
            .is_none(),
        "a future reminder must not be claimable"
    );

    let due = session
        .create_sleeping_task(
            NewTask::new("reminder", 1, "remind", json!({"message": "now"}))
                .conversation(&conversation.id),
            now - 1,
        )
        .expect("due task");
    session.recover_tasks().expect("recovery");
    let claimed = session
        .claim_task_for_kind("scheduler", 60_000_000, "reminder")
        .expect("claim")
        .expect("due reminder");
    assert_eq!(claimed.id, due.id);
    assert_eq!(claimed.kind, "reminder");
    assert_eq!(
        claimed.conversation_id.as_deref(),
        Some(conversation.id.as_str())
    );
}

#[test]
fn claiming_by_kind_leaves_other_tasks_alone() {
    let (_home, harness) = home();
    let session = harness.session();
    let now = session.now_micros();
    let other = session
        .create_task(NewTask::new("other", 1, "start", json!({})))
        .expect("other task");
    let reminder = session
        .create_sleeping_task(NewTask::new("reminder", 1, "remind", json!({})), now - 1)
        .expect("reminder");

    session.recover_tasks().expect("recovery");
    let claimed = session
        .claim_task_for_kind("scheduler", 60_000_000, "reminder")
        .expect("claim")
        .expect("reminder");
    assert_eq!(claimed.id, reminder.id);
    assert_eq!(
        session
            .task(&other.id)
            .expect("read")
            .expect("other")
            .status,
        TaskStatus::Pending
    );

    let claimed_other = session
        .claim_task_for_kind("scheduler", 60_000_000, "other")
        .expect("claim")
        .expect("other");
    assert_eq!(claimed_other.id, other.id);
}
