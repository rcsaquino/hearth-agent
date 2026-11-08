use hearth_core::{CoreError, Harness, MemoryChange, MemoryWrite, NewMemory, Save};
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
                .memory(MemoryWrite::new(MemoryChange::Add(item.clone()), "agent")),
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

#[test]
fn latest_user_entry_id_tracks_the_newest_user_entry() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    assert!(
        session
            .latest_user_entry_id(&conversation.id)
            .expect("latest")
            .is_none()
    );
    session
        .submit(&conversation.id, "req-1", "input", json!({"text": "one"}))
        .expect("first");
    let first = session
        .latest_user_entry_id(&conversation.id)
        .expect("latest first")
        .expect("first entry");
    session
        .submit(&conversation.id, "req-2", "input", json!({"text": "two"}))
        .expect("second");
    let second = session
        .latest_user_entry_id(&conversation.id)
        .expect("latest second")
        .expect("second entry");
    assert_ne!(first, second);
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(second, history.last().expect("last entry").id);
    assert_eq!(history.last().expect("last entry").kind, "user");
}

#[test]
fn a_save_updates_meta_only_when_the_expected_value_matches() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");

    session
        .save(Save::new(&conversation.id).meta("standing_version", None, "1"))
        .expect("create meta");
    assert_eq!(
        session.meta_value("standing_version").expect("read"),
        Some("1".to_owned())
    );

    let error = session
        .save(Save::new(&conversation.id).meta("standing_version", Some("0".to_owned()), "2"))
        .expect_err("stale meta must fail");
    assert!(matches!(error, CoreError::MetaVersion { .. }));
    assert_eq!(
        session.meta_value("standing_version").expect("read"),
        Some("1".to_owned())
    );

    session
        .save(Save::new(&conversation.id).meta("standing_version", Some("1".to_owned()), "2"))
        .expect("update meta");
    assert_eq!(
        session.meta_value("standing_version").expect("read"),
        Some("2".to_owned())
    );
}

#[test]
fn a_meta_mismatch_rolls_back_a_memory_write() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let item = memory(session, "Sam likes coffee", "fp-coffee");

    let error = session
        .save(
            Save::new(&conversation.id)
                .memory(MemoryWrite::new(MemoryChange::Add(item), "agent"))
                .meta("standing_version", Some("9".to_owned()), "10"),
        )
        .expect_err("meta mismatch must abort");
    assert!(matches!(error, CoreError::MetaVersion { .. }));
    assert_eq!(count(session, "SELECT COUNT(*) FROM memory_items"), 0);
    assert_eq!(
        session.meta_value("standing_version").expect("read"),
        None
    );
}
