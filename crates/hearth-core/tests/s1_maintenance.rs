use hearth_core::{Harness, NewTask, SubmissionStatus};
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

#[test]
fn a_meta_write_needs_no_conversation() {
    let (_home, harness) = harness();
    let session = harness.session();
    assert!(session.meta_value("last_review").expect("read").is_none());
    session
        .set_meta("last_review", "2026-11-09T04:30:00Z")
        .expect("set meta");
    assert_eq!(
        session.meta_value("last_review").expect("read"),
        Some("2026-11-09T04:30:00Z".to_owned())
    );
    session.set_meta("last_review", "2026-11-10T04:30:00Z").expect("overwrite");
    assert_eq!(
        session.meta_value("last_review").expect("read"),
        Some("2026-11-10T04:30:00Z".to_owned())
    );
}

#[test]
fn open_tasks_are_detected_by_kind() {
    let (_home, harness) = harness();
    let session = harness.session();
    assert!(!session.has_open_task("skill.review").expect("lookup"));
    session
        .create_task(NewTask::new(
            "skill.review",
            1,
            "run",
            json!({"slot": "04:30"}),
        ))
        .expect("create task");
    assert!(session.has_open_task("skill.review").expect("lookup"));
    assert!(!session.has_open_task("memory.consolidate").expect("lookup"));
}

#[test]
fn settled_submissions_are_listed_after_a_cutoff() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Maintenance", "local", "local:maintenance")
        .expect("conversation");
    let submitted = session
        .submit(
            &conversation.id,
            "maintenance-1",
            "input",
            json!({"text": "settle me"}),
        )
        .expect("submit");
    session
        .settle(&submitted.submission.id, SubmissionStatus::Done, None)
        .expect("settle");
    let settled = session
        .submissions_settled_since(0)
        .expect("settled submissions");
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].id, submitted.submission.id);
    assert!(settled[0].settled_at.is_some());
    let future = session.now_micros() + 1_000_000;
    assert!(
        session
            .submissions_settled_since(future)
            .expect("future cutoff")
            .is_empty()
    );
}
