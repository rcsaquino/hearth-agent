use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use hearth_core::{Clock, Harness, IdGen, NewTask, StepOutput, TaskStatus, TerminalOutcome};
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
