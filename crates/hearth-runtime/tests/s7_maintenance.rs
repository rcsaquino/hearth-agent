use hearth_core::{Harness, Zone};
use hearth_runtime::maintenance::ensure_maintenance_tasks;
use tempfile::TempDir;
use time::OffsetDateTime;

fn micros(value: &str) -> i64 {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .expect("timestamp")
        .unix_timestamp_nanos()
        .div_euclid(1_000) as i64
}

#[test]
fn daily_maintenance_wakes_at_the_local_slot() {
    let zone = Zone::fixed(8 * 3600);
    let before = micros("2026-11-09T02:00:00+08:00");
    let next = zone.next_at(before, 3, 30);
    assert_eq!(next, micros("2026-11-09T03:30:00+08:00"));
    let after = micros("2026-11-09T04:00:00+08:00");
    let next = zone.next_at(after, 3, 30);
    assert_eq!(next, micros("2026-11-10T03:30:00+08:00"));
    let review = zone.next_at(before, 4, 30);
    assert_eq!(review, micros("2026-11-09T04:30:00+08:00"));
}

#[test]
fn the_maintenance_tasks_are_created_once() {
    let home = TempDir::new().expect("home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    ensure_maintenance_tasks(session, &Zone::utc(), 30, 25).expect("ensure");
    assert!(session.has_open_task("backup").expect("backup"));
    assert!(session.has_open_task("skill.review").expect("skill"));
    assert!(
        session
            .has_open_task("helper.retention")
            .expect("helper retention")
    );
    // Memory has no scheduled pass: the standing file is repaired at startup.
    assert!(
        !session
            .has_open_task("memory.consolidate")
            .expect("memory task lookup")
    );
    let counts = session.task_counts().expect("counts");
    assert_eq!(counts.asleep, 3);
    ensure_maintenance_tasks(session, &Zone::utc(), 30, 25).expect("ensure again");
    let counts = session.task_counts().expect("counts");
    assert_eq!(counts.asleep, 3);
}
