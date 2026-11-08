use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use hearth_core::{Clock, Harness, IdGen, NewTask, Save, StepOutput, TaskStatus, TerminalOutcome};
use serde_json::json;
use tempfile::TempDir;

#[derive(Clone)]
struct TestClock(Arc<AtomicI64>);

impl TestClock {
    fn new(start: i64) -> Self {
        Self(Arc::new(AtomicI64::new(start)))
    }

    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    fn advance(&self, micros: i64) {
        self.0.fetch_add(micros, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_micros(&self) -> i64 {
        self.now()
    }
}

struct TestIds(Arc<AtomicU64>);

impl TestIds {
    fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }
}

impl IdGen for TestIds {
    fn new_id(&self) -> String {
        format!("id-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

fn harness() -> (TempDir, Harness, TestClock) {
    let home = TempDir::new().expect("temp home");
    let clock = TestClock::new(1_000_000);
    let harness = Harness::open_with(
        home.path(),
        Arc::new(clock.clone()),
        Arc::new(TestIds::new()),
    )
    .expect("open harness");
    (home, harness, clock)
}

#[test]
fn a_new_task_is_pending_and_readable() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");

    assert_eq!(task.status, TaskStatus::Pending);
    assert_eq!(task.attempt, 0);
    assert_eq!(task.phase, "start");
    assert_eq!(task.input["n"], 1);
    assert_eq!(task.version, 1);
    assert!(task.lease_owner.is_none());
    assert!(task.lease_until.is_none());
    assert!(task.next_wake_at.is_none());
    assert!(task.terminal_at.is_none());
    assert_eq!(task.created_at, clock.now());

    let read = session
        .task(&task.id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(read, task);
}

#[test]
fn a_claim_marks_a_task_running_with_a_lease_and_attempt() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");

    let claimed = session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");
    assert_eq!(claimed.id, task.id);
    assert_eq!(claimed.phase, "start");
    assert_eq!(claimed.attempt, 1);
    assert_eq!(claimed.input["n"], 1);

    let running = session
        .task(&task.id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(running.status, TaskStatus::Running);
    assert_eq!(running.lease_owner.as_deref(), Some("worker-a"));
    assert_eq!(running.lease_until, Some(clock.now() + 60_000_000));
    assert_eq!(running.attempt, 1);
}

#[test]
fn a_second_claim_cannot_take_a_leased_running_task() {
    let (_home, harness, _clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");

    let again = session
        .claim_task("worker-b", 60_000_000)
        .expect("second claim");
    assert!(again.is_none());

    let running = session
        .task(&task.id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(running.attempt, 1);
    assert_eq!(running.lease_owner.as_deref(), Some("worker-a"));
}

#[test]
fn next_releases_the_lease_and_requeues_the_next_phase() {
    let (_home, harness, _clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    let claimed = session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");

    let next = session
        .finish_task(
            &claimed.id,
            "worker-a",
            StepOutput::Next {
                phase: "second".to_owned(),
                checkpoint: json!({"step": 1}),
            },
        )
        .expect("finish step");
    assert_eq!(next.status, TaskStatus::Pending);
    assert_eq!(next.phase, "second");
    assert_eq!(next.checkpoint, Some(json!({"step": 1})));
    assert_eq!(next.attempt, 0);
    assert!(next.lease_owner.is_none());
    assert!(next.lease_until.is_none());
    assert!(next.next_wake_at.is_none());
    assert!(next.terminal_at.is_none());

    let again = session
        .claim_task("worker-b", 60_000_000)
        .expect("claim next phase")
        .expect("the task is runnable again");
    assert_eq!(again.id, task.id);
    assert_eq!(again.phase, "second");
    assert_eq!(again.attempt, 1);
    assert_eq!(again.checkpoint, Some(json!({"step": 1})));
}

#[test]
fn sleep_sets_the_wake_time_and_recovery_requeues_it_when_due() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    let claimed = session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");
    let wake = clock.now() + 100_000_000;

    let sleeping = session
        .finish_task(
            &claimed.id,
            "worker-a",
            StepOutput::Sleep {
                until_micros: wake,
                phase: "wait".to_owned(),
                checkpoint: json!({"step": 2}),
            },
        )
        .expect("sleep");
    assert_eq!(sleeping.status, TaskStatus::Asleep);
    assert_eq!(sleeping.next_wake_at, Some(wake));
    assert_eq!(sleeping.attempt, 0);
    assert!(sleeping.lease_owner.is_none());
    assert!(sleeping.lease_until.is_none());

    assert!(
        session
            .claim_task("worker-b", 60_000_000)
            .expect("claim while asleep")
            .is_none()
    );

    clock.advance(50_000_000);
    let early = session.recover_tasks().expect("early recovery");
    assert_eq!(early.woken, 0);
    assert_eq!(
        session
            .task(&task.id)
            .expect("read")
            .expect("exists")
            .status,
        TaskStatus::Asleep
    );

    clock.advance(50_000_000);
    let due = session.recover_tasks().expect("due recovery");
    assert_eq!(due.woken, 1);
    let woken = session.task(&task.id).expect("read").expect("exists");
    assert_eq!(woken.status, TaskStatus::Pending);
    assert!(woken.next_wake_at.is_none());

    let claimed = session
        .claim_task("worker-c", 60_000_000)
        .expect("claim after wake")
        .expect("the task is runnable");
    assert_eq!(claimed.phase, "wait");
    assert_eq!(claimed.checkpoint, Some(json!({"step": 2})));
}

#[test]
fn a_terminal_outcome_settles_the_task_once() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    let claimed = session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");
    let finished = session
        .finish_task(
            &claimed.id,
            "worker-a",
            StepOutput::Terminal {
                outcome: TerminalOutcome::Done,
                result: json!({"ok": true}),
            },
        )
        .expect("terminal step");
    assert_eq!(finished.status, TaskStatus::Done);
    assert_eq!(finished.result, Some(json!({"ok": true})));
    assert_eq!(finished.terminal_at, Some(clock.now()));
    assert!(finished.lease_owner.is_none());
    assert!(finished.lease_until.is_none());
    assert!(finished.next_wake_at.is_none());

    let second = session.finish_task(
        &claimed.id,
        "worker-a",
        StepOutput::Terminal {
            outcome: TerminalOutcome::Done,
            result: json!({"again": true}),
        },
    );
    assert!(matches!(
        second,
        Err(hearth_core::CoreError::TaskNotRunning(_, _))
    ));
    let report = session.recover_tasks().expect("recovery");
    assert_eq!(report.requeued, 0);
    assert_eq!(
        session
            .task(&task.id)
            .expect("read")
            .expect("exists")
            .status,
        TaskStatus::Done
    );
}

#[test]
fn a_retryable_failure_requeues_with_a_delay_and_resets_attempt() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    let claimed = session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");
    let retry_at = clock.now() + 1_000_000;

    let retried = session
        .finish_task(
            &claimed.id,
            "worker-a",
            StepOutput::Failed {
                error: json!({"code": "temporary"}),
                retryable: true,
            },
        )
        .expect("retryable failure");
    assert_eq!(retried.status, TaskStatus::Pending);
    assert_eq!(retried.next_wake_at, Some(retry_at));
    assert_eq!(retried.attempt, 0);
    assert!(retried.result.is_none());
    assert!(retried.lease_owner.is_none());

    assert!(
        session
            .claim_task("worker-b", 60_000_000)
            .expect("early claim")
            .is_none()
    );
    clock.advance(1_000_000);
    let claimed = session
        .claim_task("worker-b", 60_000_000)
        .expect("claim after delay")
        .expect("the task is runnable again");
    assert_eq!(claimed.id, task.id);
    assert_eq!(claimed.attempt, 1);
}

#[test]
fn a_non_retryable_failure_settles_the_task_as_failed() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    let claimed = session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");
    let failed = session
        .finish_task(
            &claimed.id,
            "worker-a",
            StepOutput::Failed {
                error: json!({"code": "fatal"}),
                retryable: false,
            },
        )
        .expect("non-retryable failure");
    assert_eq!(failed.status, TaskStatus::Failed);
    assert_eq!(failed.result, Some(json!({"code": "fatal"})));
    assert_eq!(failed.terminal_at, Some(clock.now()));
    assert!(failed.lease_owner.is_none());
    assert!(failed.lease_until.is_none());
    assert!(failed.next_wake_at.is_none());

    assert!(
        session
            .claim_task("worker-b", 60_000_000)
            .expect("claim failed task")
            .is_none()
    );
    assert_eq!(
        session
            .task(&task.id)
            .expect("read")
            .expect("exists")
            .status,
        TaskStatus::Failed
    );
}

#[test]
fn an_expired_lease_is_requeued_on_recovery() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    session
        .claim_task("worker-a", 1_000_000)
        .expect("claim task")
        .expect("a task is runnable");
    clock.advance(2_000_000);

    let report = session.recover_tasks().expect("recovery");
    assert_eq!(report.requeued, 1);
    assert_eq!(report.poisoned, 0);
    let recovered = session.task(&task.id).expect("read").expect("exists");
    assert_eq!(recovered.status, TaskStatus::Pending);
    assert!(recovered.lease_owner.is_none());
    assert!(recovered.lease_until.is_none());
    assert_eq!(recovered.attempt, 1);

    let claimed = session
        .claim_task("worker-b", 60_000_000)
        .expect("claim after recovery")
        .expect("the task is runnable");
    assert_eq!(claimed.attempt, 2);
}

#[test]
fn a_task_that_expires_twice_is_failed_as_poisoned() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    session
        .claim_task("worker-a", 1_000_000)
        .expect("first claim")
        .expect("a task is runnable");
    clock.advance(2_000_000);
    let first = session.recover_tasks().expect("first recovery");
    assert_eq!(first.requeued, 1);
    assert_eq!(first.poisoned, 0);

    session
        .claim_task("worker-b", 1_000_000)
        .expect("second claim")
        .expect("the task is runnable again");
    clock.advance(2_000_000);
    let second = session.recover_tasks().expect("second recovery");
    assert_eq!(second.requeued, 0);
    assert_eq!(second.poisoned, 1);

    let poisoned = session.task(&task.id).expect("read").expect("exists");
    assert_eq!(poisoned.status, TaskStatus::Failed);
    assert_eq!(poisoned.result, Some(json!({"error": "poisoned"})));
    assert_eq!(poisoned.terminal_at, Some(clock.now()));
    assert!(
        session
            .claim_task("worker-c", 60_000_000)
            .expect("claim poisoned")
            .is_none()
    );
}

#[test]
fn a_heartbeat_extends_the_lease_and_a_stale_owner_is_rejected() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("a task is runnable");
    let first_lease = session
        .task(&task.id)
        .expect("read")
        .expect("exists")
        .lease_until
        .expect("leased");

    clock.advance(30_000_000);
    session
        .heartbeat_task(&task.id, "worker-a", 60_000_000)
        .expect("heartbeat");
    let extended = session
        .task(&task.id)
        .expect("read")
        .expect("exists")
        .lease_until
        .expect("leased");
    assert_eq!(extended, clock.now() + 60_000_000);
    assert!(extended > first_lease);

    let stale = session.heartbeat_task(&task.id, "worker-b", 60_000_000);
    assert!(matches!(stale, Err(hearth_core::CoreError::LeaseLost(_))));
    let stale_finish = session.finish_task(
        &task.id,
        "worker-b",
        StepOutput::Next {
            phase: "other".to_owned(),
            checkpoint: json!({}),
        },
    );
    assert!(matches!(
        stale_finish,
        Err(hearth_core::CoreError::LeaseLost(_))
    ));
    assert_eq!(
        session
            .task(&task.id)
            .expect("read")
            .expect("exists")
            .lease_owner
            .as_deref(),
        Some("worker-a")
    );

    session
        .finish_task(
            &task.id,
            "worker-a",
            StepOutput::Next {
                phase: "done".to_owned(),
                checkpoint: json!({}),
            },
        )
        .expect("the owner finishes");
}

#[test]
fn a_recovered_claim_continues_from_the_saved_checkpoint_and_phase() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    let claimed = session
        .claim_task("worker-a", 60_000_000)
        .expect("claim")
        .expect("runnable");
    session
        .finish_task(
            &claimed.id,
            "worker-a",
            StepOutput::Next {
                phase: "second".to_owned(),
                checkpoint: json!({"step": 1}),
            },
        )
        .expect("next step");

    session
        .claim_task("worker-b", 1_000_000)
        .expect("claim second phase")
        .expect("runnable");
    clock.advance(2_000_000);
    session.recover_tasks().expect("recovery");

    let resumed = session
        .claim_task("worker-c", 60_000_000)
        .expect("claim after recovery")
        .expect("runnable");
    assert_eq!(resumed.id, task.id);
    assert_eq!(resumed.phase, "second");
    assert_eq!(resumed.checkpoint, Some(json!({"step": 1})));
    assert_eq!(resumed.attempt, 2);
}

#[test]
fn tasks_are_claimed_in_wake_order_with_creation_order_breaking_ties() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let first = session
        .create_task(NewTask::new("first", 1, "start", json!({})))
        .expect("create first");
    let second = session
        .create_task(NewTask::new("second", 1, "start", json!({})))
        .expect("create second");
    session
        .claim_task("worker-a", 60_000_000)
        .expect("claim first")
        .expect("runnable");
    session
        .claim_task("worker-a", 60_000_000)
        .expect("claim second")
        .expect("runnable");

    session
        .finish_task(
            &second.id,
            "worker-a",
            StepOutput::Failed {
                error: json!({"code": "temporary"}),
                retryable: true,
            },
        )
        .expect("fail second early");
    clock.advance(10_000_000);
    session
        .finish_task(
            &first.id,
            "worker-a",
            StepOutput::Failed {
                error: json!({"code": "temporary"}),
                retryable: true,
            },
        )
        .expect("fail first later");

    clock.advance(1_000_000);
    let claimed = session
        .claim_task("worker-b", 60_000_000)
        .expect("claim wake order")
        .expect("runnable");
    assert_eq!(claimed.id, second.id, "the earlier wake wins");
    session
        .finish_task(
            &second.id,
            "worker-b",
            StepOutput::Terminal {
                outcome: TerminalOutcome::Done,
                result: json!({}),
            },
        )
        .expect("finish second");
    let claimed = session
        .claim_task("worker-b", 60_000_000)
        .expect("claim first")
        .expect("runnable");
    assert_eq!(claimed.id, first.id);
    session
        .finish_task(
            &first.id,
            "worker-b",
            StepOutput::Terminal {
                outcome: TerminalOutcome::Done,
                result: json!({}),
            },
        )
        .expect("finish first");

    let third = session
        .create_task(NewTask::new("third", 1, "start", json!({})))
        .expect("create third");
    let fourth = session
        .create_task(NewTask::new("fourth", 1, "start", json!({})))
        .expect("create fourth");
    session
        .claim_task("worker-c", 60_000_000)
        .expect("claim third")
        .expect("runnable");
    session
        .claim_task("worker-c", 60_000_000)
        .expect("claim fourth")
        .expect("runnable");
    session
        .finish_task(
            &third.id,
            "worker-c",
            StepOutput::Failed {
                error: json!({"code": "temporary"}),
                retryable: true,
            },
        )
        .expect("fail third");
    session
        .finish_task(
            &fourth.id,
            "worker-c",
            StepOutput::Failed {
                error: json!({"code": "temporary"}),
                retryable: true,
            },
        )
        .expect("fail fourth");

    clock.advance(1_000_000);
    let claimed = session
        .claim_task("worker-d", 60_000_000)
        .expect("claim tie")
        .expect("runnable");
    assert_eq!(claimed.id, third.id, "creation order breaks the tie");
}

#[test]
fn a_task_for_an_unknown_conversation_or_parent_task_is_rejected() {
    let (_home, harness, _clock) = harness();
    let session = harness.session();

    let missing_conversation = session
        .create_task(NewTask::new("test", 1, "start", json!({})).conversation("missing"))
        .unwrap_err();
    assert!(matches!(
        missing_conversation,
        hearth_core::CoreError::ConversationNotFound(_)
    ));

    let missing_parent = session
        .create_task(NewTask::new("test", 1, "start", json!({})).parent("missing"))
        .unwrap_err();
    assert!(matches!(
        missing_parent,
        hearth_core::CoreError::TaskNotFound(_)
    ));

    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let parent = session
        .create_task(NewTask::new("parent", 1, "start", json!({})))
        .expect("create parent");
    let child = session
        .create_task(
            NewTask::new("child", 1, "start", json!({}))
                .conversation(conversation.id.clone())
                .parent(parent.id.clone()),
        )
        .expect("create child");
    assert_eq!(
        child.conversation_id.as_deref(),
        Some(conversation.id.as_str())
    );
    assert_eq!(child.parent_task_id.as_deref(), Some(parent.id.as_str()));
}

#[test]
fn the_latest_commit_is_none_on_a_new_harness() {
    let (_home, harness, _clock) = harness();

    assert!(
        harness
            .session()
            .latest_commit()
            .expect("latest commit")
            .is_none()
    );
}

#[test]
fn a_save_records_the_latest_commit() {
    let (_home, harness, clock) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    let commit = session
        .save(Save::new(&conversation.id).entry("user", json!({"text": "hello"})))
        .expect("save");

    let latest = session
        .latest_commit()
        .expect("latest commit")
        .expect("a save records a commit");
    assert_eq!(latest.seq, commit.seq);
    assert_eq!(latest.created_at, clock.now());
    assert_eq!(latest.kind, "save");
    assert_eq!(
        latest.conversation_id.as_deref(),
        Some(conversation.id.as_str())
    );
    assert!(latest.task_id.is_none());
}

#[test]
fn creating_a_task_records_a_task_commit() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");

    let latest = session
        .latest_commit()
        .expect("latest commit")
        .expect("creating a task records a commit");
    assert_eq!(latest.seq, 1);
    assert_eq!(latest.kind, "task.create");
    assert_eq!(latest.task_id.as_deref(), Some(task.id.as_str()));
    assert!(latest.conversation_id.is_none());
    assert_eq!(latest.created_at, clock.now());
}

#[test]
fn claiming_a_task_records_a_claim_commit() {
    let (_home, harness, clock) = harness();
    let session = harness.session();

    let task = session
        .create_task(NewTask::new("test", 1, "start", json!({"n": 1})))
        .expect("create task");
    session
        .claim_task("worker-a", 60_000_000)
        .expect("claim task")
        .expect("task is runnable");

    let latest = session
        .latest_commit()
        .expect("latest commit")
        .expect("a claim records a commit");
    assert_eq!(latest.seq, 2);
    assert_eq!(latest.kind, "task.claim");
    assert_eq!(latest.task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(latest.created_at, clock.now());
}

#[test]
fn finishing_a_task_records_the_transition_commit() {
    let cases = vec![
        (
            StepOutput::Next {
                phase: "next".to_owned(),
                checkpoint: json!({"step": 2}),
            },
            "task.next",
        ),
        (
            StepOutput::Sleep {
                until_micros: 9_000_000,
                phase: "sleep".to_owned(),
                checkpoint: json!({"step": 2}),
            },
            "task.sleep",
        ),
        (
            StepOutput::Terminal {
                outcome: TerminalOutcome::Done,
                result: json!({"ok": true}),
            },
            "task.terminal",
        ),
        (
            StepOutput::Failed {
                error: json!({"message": "again"}),
                retryable: true,
            },
            "task.retry",
        ),
        (
            StepOutput::Failed {
                error: json!({"message": "stop"}),
                retryable: false,
            },
            "task.fail",
        ),
    ];

    for (output, expected_kind) in cases {
        let (_home, harness, _clock) = harness();
        let session = harness.session();
        let task = session
            .create_task(NewTask::new("test", 1, "start", json!({})))
            .expect("create task");
        session
            .claim_task("worker-a", 60_000_000)
            .expect("claim task")
            .expect("task is runnable");
        session
            .finish_task(&task.id, "worker-a", output)
            .expect("finish task");

        let latest = session
            .latest_commit()
            .expect("latest commit")
            .expect("a finish records a commit");
        assert_eq!(latest.seq, 3, "case {expected_kind}");
        assert_eq!(latest.kind, expected_kind);
        assert_eq!(latest.task_id.as_deref(), Some(task.id.as_str()));
    }
}
