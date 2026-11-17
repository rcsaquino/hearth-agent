//! S4: capture reads the entry a run answered, not whatever arrived last.

use hearth_core::{Harness, Save, Session};
use hearth_memory::MemoryService;
use hearth_runtime::extraction::{capture, capture_entry};
use tempfile::TempDir;

fn user_entry(session: &Session, conversation_id: &str, text: &str) -> String {
    session
        .save(
            Save::new(conversation_id)
                .entry("user", serde_json::json!([{"type": "text", "text": text}])),
        )
        .expect("save")
        .entry_ids
        .first()
        .cloned()
        .expect("a saved user entry")
}

#[test]
fn an_explicit_remember_is_written_once() {
    let directory = TempDir::new().expect("temp home");
    let harness = Harness::open(directory.path()).expect("harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone());
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let entry = user_entry(
        &session,
        &conversation.id,
        "Remember that Alice manages the OR board.",
    );

    let first = capture_entry(&session, &memory, &entry).expect("capture");
    assert_eq!(first.written, 1);
    let second = capture_entry(&session, &memory, &entry).expect("capture");
    assert_eq!(second.written, 0, "a replayed capture must not duplicate");
    assert_eq!(second.skipped, 1);
    assert_eq!(memory.count().expect("count"), 1);
}

#[test]
fn a_later_message_is_not_captured_by_an_earlier_entry() {
    let directory = TempDir::new().expect("temp home");
    let harness = Harness::open(directory.path()).expect("harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone());
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let first = user_entry(
        &session,
        &conversation.id,
        "Remember that Alice manages the OR board.",
    );
    let second = user_entry(&session, &conversation.id, "Remember that Bob is my boss.");

    let report = capture_entry(&session, &memory, &first).expect("capture the run entry");
    assert_eq!(report.written, 1);
    let contents: Vec<String> = memory
        .recent(None, 10)
        .expect("recent")
        .into_iter()
        .map(|memory| memory.content)
        .collect();
    assert!(
        contents.iter().any(|content| content.contains("Alice")),
        "the finished run's entry must be captured: {contents:?}"
    );
    assert!(
        !contents.iter().any(|content| content.contains("Bob")),
        "the queued entry belongs to its own run: {contents:?}"
    );

    let later = capture_entry(&session, &memory, &second).expect("capture the second entry");
    assert_eq!(later.written, 1);
    assert_eq!(memory.count().expect("count"), 2);
}

#[test]
fn a_captured_entry_still_resolves_after_a_restart() {
    let directory = TempDir::new().expect("temp home");
    let entry = {
        let harness = Harness::open(directory.path()).expect("harness");
        let session = harness.session().clone();
        let conversation = session
            .create_conversation("Test", "local", "local:1")
            .expect("conversation");
        user_entry(
            &session,
            &conversation.id,
            "Remember that Alice manages the OR board.",
        )
    };

    let harness = Harness::open(directory.path()).expect("reopen harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone());
    let report = capture_entry(&session, &memory, &entry).expect("capture after restart");
    assert_eq!(report.written, 1);
    assert_eq!(memory.count().expect("count"), 1);
}

#[test]
fn an_ordinary_message_writes_nothing() {
    let directory = TempDir::new().expect("temp home");
    let harness = Harness::open(directory.path()).expect("harness");
    let memory = MemoryService::new(harness.session().clone());
    let report = capture(&memory, None, "can you summarise the report").expect("capture");
    assert_eq!(report.written, 0);
    assert_eq!(memory.count().expect("count"), 0);
}
