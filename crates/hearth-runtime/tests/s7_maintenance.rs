use std::sync::{Arc, Mutex};

use hearth_core::{ClaimedTask, Clock, Harness, Save, StepOutput, UuidV7};
use hearth_memory::{MemoryKind, MemoryService, NewMemoryInput, SourceKind};
use hearth_runtime::TaskHandler;
use hearth_runtime::maintenance::{
    MemoryMaintenanceHandler, ensure_maintenance_tasks, next_daily_micros,
};
use serde_json::json;
use tempfile::TempDir;
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

struct TestClock(Arc<Mutex<i64>>);

impl Clock for TestClock {
    fn now_micros(&self) -> i64 {
        *self.0.lock().expect("clock")
    }
}

fn micros(value: &str) -> i64 {
    OffsetDateTime::parse(value, &Rfc3339)
        .expect("time")
        .unix_timestamp_nanos()
        .div_euclid(1_000) as i64
}

fn local_micros(value: &str) -> i64 {
    OffsetDateTime::parse(value, &Rfc3339)
        .expect("time")
        .unix_timestamp_nanos()
        .div_euclid(1_000) as i64
}

#[test]
fn daily_maintenance_wakes_at_the_local_slot() {
    let offset = UtcOffset::from_hms(8, 0, 0).expect("offset");
    let before = micros("2026-11-09T02:00:00+08:00");
    let next = next_daily_micros(before, offset, 3, 30);
    assert_eq!(next, micros("2026-11-09T03:30:00+08:00"));
    let after = micros("2026-11-09T04:00:00+08:00");
    let next = next_daily_micros(after, offset, 3, 30);
    assert_eq!(next, micros("2026-11-10T03:30:00+08:00"));
    let review = next_daily_micros(before, offset, 4, 30);
    assert_eq!(review, micros("2026-11-09T04:30:00+08:00"));
}

#[test]
fn the_two_maintenance_tasks_are_created_once() {
    let home = TempDir::new().expect("home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    ensure_maintenance_tasks(session).expect("ensure");
    assert!(session.has_open_task("memory.consolidate").expect("memory"));
    assert!(session.has_open_task("skill.review").expect("skill"));
    let counts = session.task_counts().expect("counts");
    assert_eq!(counts.asleep, 2);
    ensure_maintenance_tasks(session).expect("ensure again");
    let counts = session.task_counts().expect("counts");
    assert_eq!(counts.asleep, 2);
}

#[test]
fn the_memory_maintenance_handler_consolidates_and_schedules_the_next_night() {
    let home = TempDir::new().expect("home");
    let clock = Arc::new(Mutex::new(micros("2022-01-01T00:00:00Z")));
    let harness = Harness::open_with(
        home.path(),
        Arc::new(TestClock(Arc::clone(&clock))),
        Arc::new(UuidV7),
    )
    .expect("harness");
    let session = harness.session();
    let memory = Arc::new(MemoryService::new(session.clone()));
    memory
        .save(NewMemoryInput {
            content: "The old lab notebook is on the top shelf.".to_owned(),
            kind: MemoryKind::Fact,
            tags: vec!["lab".to_owned()],
            importance: 10,
            source_kind: SourceKind::User,
            source_ref: None,
            supersedes: None,
            valid_from: None,
            valid_to: None,
        })
        .expect("old memory");
    let conversation = session
        .create_conversation("Episode", "local", "local:episode")
        .expect("conversation");
    session
        .save(Save::new(&conversation.id).entry("user", json!({"text": "We moved the bench."})))
        .expect("entry");
    *clock.lock().expect("clock") = micros("2026-11-09T02:00:00Z");
    let memory = Arc::new(MemoryService::new(session.clone()));
    let handler = MemoryMaintenanceHandler::new(session.clone(), memory);
    let task = ClaimedTask {
        id: "task-memory".to_owned(),
        conversation_id: None,
        kind: "memory.consolidate".to_owned(),
        phase: "run".to_owned(),
        attempt: 1,
        input: json!({"slot": "03:30"}),
        checkpoint: None,
    };
    let output = handler.step(&task).expect("maintenance");
    match output {
        StepOutput::Sleep { until_micros, .. } => {
            assert!(until_micros > session.now_micros());
        }
        other => panic!("expected sleep, got {other:?}"),
    }
    assert!(
        session
            .meta_value("last_consolidation")
            .expect("meta")
            .is_some()
    );
    let stats = MemoryService::new(session.clone()).stats().expect("stats");
    assert_eq!(stats.by_kind.get("episode").copied().unwrap_or(0), 1);
}

#[test]
fn the_memory_maintenance_reports_the_next_local_slot() {
    let home = TempDir::new().expect("home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let memory = Arc::new(MemoryService::new(session.clone()));
    let handler = MemoryMaintenanceHandler::new(session.clone(), memory);
    let task = ClaimedTask {
        id: "task-memory".to_owned(),
        conversation_id: None,
        kind: "memory.consolidate".to_owned(),
        phase: "run".to_owned(),
        attempt: 1,
        input: json!({"slot": "03:30"}),
        checkpoint: None,
    };
    let output = handler.step(&task).expect("maintenance");
    let StepOutput::Sleep { until_micros, .. } = output else {
        panic!("expected sleep");
    };
    let wake = OffsetDateTime::from_unix_timestamp_nanos(until_micros as i128 * 1_000)
        .expect("wake")
        .to_offset(UtcOffset::UTC);
    assert!(wake.year() >= 2026);
    let _ = local_micros("2026-11-09T03:30:00+08:00");
}
