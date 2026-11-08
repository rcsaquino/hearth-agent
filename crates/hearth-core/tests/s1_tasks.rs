use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use hearth_core::{Clock, Harness, IdGen, NewTask, TaskStatus};
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
