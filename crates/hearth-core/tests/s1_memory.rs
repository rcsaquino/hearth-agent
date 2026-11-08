use hearth_core::{
    Harness, MemoryChange, MemoryWrite, NewMemory, Save,
};
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

fn memory(session: &hearth_core::Session, content: &str, fingerprint: &str) -> NewMemory {
    NewMemory {
        id: session.new_id(),
        kind: "fact".to_owned(),
        content: content.to_owned(),
        tags: vec!["preference".to_owned()],
        fingerprint: fingerprint.to_owned(),
        importance: 70,
        status: "active".to_owned(),
        valid_from: None,
        valid_to: None,
        supersedes_id: None,
        source_kind: "user".to_owned(),
        source_ref: Some("entry-1".to_owned()),
        standing: "none".to_owned(),
        created_at: session.now_micros(),
        last_used_at: None,
    }
}

fn count(session: &hearth_core::Session, sql: &str) -> i64 {
    session
        .storage()
        .read(|reader| reader.query_row(sql, [], |row| row.get(0)))
        .expect("count")
}

#[test]
fn a_memory_add_and_an_entry_share_one_save() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let item = memory(session, "Sam likes coffee", "fp-coffee");

    let commit = session
        .save(
            Save::new(&conversation.id)
                .entry("user", json!({"text": "remember coffee"}))
                .memory(MemoryWrite::new(
                    MemoryChange::Add(item.clone()),
                    "agent",
                )),
        )
        .expect("save");

    assert_eq!(commit.entry_ids.len(), 1);
    assert_eq!(count(session, "SELECT COUNT(*) FROM memory_items"), 1);
    assert_eq!(count(session, "SELECT COUNT(*) FROM memory_events"), 1);
    assert_eq!(
        count(
            session,
            "SELECT COUNT(*) FROM memory_fts WHERE memory_fts MATCH 'coffee'"
        ),
        1
    );
}

#[test]
fn a_failed_save_leaves_no_memory_row_or_event() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let item = memory(session, "Sam likes coffee", "fp-coffee");

    let error = session
        .save(
            Save::new(&conversation.id)
                .document("agent", Some(99), json!({"model": "bad"}))
                .memory(MemoryWrite::new(MemoryChange::Add(item), "agent")),
        )
        .expect_err("stale document version must fail");
    assert!(matches!(
        error,
        hearth_core::CoreError::DocumentVersion { .. }
    ));
    assert_eq!(count(session, "SELECT COUNT(*) FROM memory_items"), 0);
    assert_eq!(count(session, "SELECT COUNT(*) FROM memory_events"), 0);
}

#[test]
fn forget_removes_the_search_row_and_keeps_the_tombstone() {
    let (_home, harness) = harness();
    let session = harness.session();
    let item = memory(session, "Sam likes coffee", "fp-coffee");
    session
        .save_memory(vec![MemoryWrite::new(
            MemoryChange::Add(item.clone()),
            "agent",
        )])
        .expect("add");
    session
        .save_memory(vec![MemoryWrite::new(
            MemoryChange::Forget {
                id: item.id.clone(),
            },
            "agent",
        )])
        .expect("forget");

    assert_eq!(
        count(
            session,
            "SELECT COUNT(*) FROM memory_fts WHERE memory_fts MATCH 'coffee'"
        ),
        0
    );
    assert_eq!(count(session, "SELECT COUNT(*) FROM memory_items"), 1);
    assert_eq!(
        count(
            session,
            "SELECT COUNT(*) FROM memory_items WHERE status = 'deleted'"
        ),
        1
    );
    assert_eq!(count(session, "SELECT COUNT(*) FROM memory_events"), 2);
}
