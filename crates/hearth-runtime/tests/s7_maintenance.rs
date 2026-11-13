use hearth_core::Harness;
use hearth_runtime::maintenance::{ensure_maintenance_tasks, next_daily_micros};
use tempfile::TempDir;
use time::OffsetDateTime;
use time::UtcOffset;

fn micros(value: &str) -> i64 {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .expect("timestamp")
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
fn the_maintenance_tasks_are_created_once() {
    let home = TempDir::new().expect("home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    ensure_maintenance_tasks(session).expect("ensure");
    assert!(session.has_open_task("backup").expect("backup"));
    assert!(session.has_open_task("skill.review").expect("skill"));
    // Memory has no scheduled pass: the standing file is repaired at startup.
    assert!(
        !session
            .has_open_task("memory.consolidate")
            .expect("memory task lookup")
    );
    let counts = session.task_counts().expect("counts");
    assert_eq!(counts.asleep, 2);
    ensure_maintenance_tasks(session).expect("ensure again");
    let counts = session.task_counts().expect("counts");
    assert_eq!(counts.asleep, 2);
}
