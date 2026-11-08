use hearth_core::{CoreError, Harness, Save, SubmissionStatus};
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

#[test]
fn a_save_appends_entries_in_order() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    let commit = session
        .save(Save::new(&conversation.id).entry("user", json!({"text": "hello"})))
        .expect("save");

    assert_eq!(commit.entry_ids.len(), 1);
    assert_eq!(commit.seq, 1);
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[0].content["text"], "hello");
}

#[test]
fn a_document_create_and_update_increments_the_version() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    session
        .save(Save::new(&conversation.id).document("agent", None, json!({"model": "one"})))
        .expect("create document");
    let first = session
        .document(&conversation.id, "agent")
        .expect("read document")
        .expect("document exists");
    assert_eq!(first.version, 1);
    assert_eq!(first.json["model"], "one");

    session
        .save(Save::new(&conversation.id).document("agent", Some(1), json!({"model": "two"})))
        .expect("update document");
    let second = session
        .document(&conversation.id, "agent")
        .expect("read document")
        .expect("document exists");
    assert_eq!(second.version, 2);
    assert_eq!(second.json["model"], "two");
}

#[test]
fn a_stale_document_version_aborts_the_whole_save() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    session
        .save(Save::new(&conversation.id).document("agent", None, json!({"model": "one"})))
        .expect("create document");

    let error = session
        .save(
            Save::new(&conversation.id)
                .entry("user", json!({"text": "lost"}))
                .document("agent", Some(0), json!({"model": "bad"})),
        )
        .expect_err("stale version must fail");
    assert!(matches!(
        error,
        hearth_core::CoreError::DocumentVersion { .. }
    ));

    assert!(
        session
            .history(&conversation.id)
            .expect("history")
            .is_empty()
    );
    let document = session
        .document(&conversation.id, "agent")
        .expect("read document")
        .expect("document exists");
    assert_eq!(document.version, 1);

    let commit = session
        .save(Save::new(&conversation.id).entry("user", json!({"text": "kept"})))
        .expect("save after failure");
    assert_eq!(commit.seq, 2);
}

#[test]
fn two_entries_in_one_save_share_the_sequence_and_keep_order() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    let commit = session
        .save(
            Save::new(&conversation.id)
                .entry("user", json!({"n": 1}))
                .entry("assistant", json!({"n": 2})),
        )
        .expect("save");

    assert_eq!(commit.entry_ids.len(), 2);
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].seq, commit.seq);
    assert_eq!(history[1].seq, commit.seq);
    assert_eq!(history[0].content["n"], 1);
    assert_eq!(history[1].content["n"], 2);
}

#[test]
fn a_duplicate_request_id_returns_the_original_and_creates_nothing() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    let first = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
        .expect("first submit");
    assert!(first.created);

    let second = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
        .expect("duplicate submit");
    assert!(!second.created);
    assert_eq!(first.submission.id, second.submission.id);

    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, "user");
}

#[test]
fn a_submission_settles_once_and_keeps_its_first_ending() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let submitted = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
        .expect("submit");

    let settled = session
        .settle(
            &submitted.submission.id,
            hearth_core::SubmissionStatus::Done,
            None,
        )
        .expect("settle");
    assert_eq!(settled.status, hearth_core::SubmissionStatus::Done);
    assert!(settled.settled_at.is_some());

    let error = session
        .settle(
            &submitted.submission.id,
            hearth_core::SubmissionStatus::Cancelled,
            None,
        )
        .expect_err("second settle must fail");
    assert!(matches!(error, hearth_core::CoreError::AlreadySettled(..)));

    let after = session
        .submission_by_id(&submitted.submission.id)
        .expect("read submission")
        .expect("submission exists");
    assert_eq!(after.status, hearth_core::SubmissionStatus::Done);
    assert_eq!(after.settled_at, settled.settled_at);
}

#[test]
fn a_fork_copies_entries_up_to_the_chosen_entry_and_links_the_parent() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    session
        .save(Save::new(&conversation.id).entry("user", json!({"n": 1})))
        .expect("first save");
    session
        .save(Save::new(&conversation.id).entry("assistant", json!({"n": 2})))
        .expect("second save");
    let history = session.history(&conversation.id).expect("history");
    let chosen = &history[1];

    let fork = session.fork(&conversation.id, &chosen.id).expect("fork");
    assert_eq!(fork.parent_id.as_deref(), Some(conversation.id.as_str()));
    assert_eq!(fork.parent_entry_id.as_deref(), Some(chosen.id.as_str()));

    let fork_history = session.history(&fork.id).expect("fork history");
    assert_eq!(fork_history.len(), 2);
    assert_eq!(fork_history[0].content["n"], 1);
    assert_eq!(fork_history[1].content["n"], 2);
    assert_eq!(fork_history[0].seq, fork_history[1].seq);
    assert_ne!(fork_history[0].id, history[0].id);
    assert_eq!(session.history(&conversation.id).expect("source").len(), 2);
}

#[test]
fn reopening_the_harness_preserves_the_records() {
    let home = TempDir::new().expect("temp home");
    let (conversation_id, submission_id) = {
        let harness = Harness::open(home.path()).expect("open harness");
        let session = harness.session();
        let conversation = session
            .create_conversation("Test", "local", "local:1")
            .expect("create conversation");
        session
            .save(
                Save::new(&conversation.id)
                    .entry("user", json!({"n": 1}))
                    .document("agent", None, json!({"model": "one"})),
            )
            .expect("save");
        let submitted = session
            .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
            .expect("submit");
        (conversation.id, submitted.submission.id)
    };

    let harness = Harness::open(home.path()).expect("reopen harness");
    let session = harness.session();
    assert_eq!(session.history(&conversation_id).expect("history").len(), 2);
    let document = session
        .document(&conversation_id, "agent")
        .expect("read document")
        .expect("document exists");
    assert_eq!(document.version, 1);
    let submission = session
        .submission_by_request_id("req-1")
        .expect("read submission")
        .expect("submission exists");
    assert_eq!(submission.id, submission_id);
}

#[test]
fn concurrent_saves_commit_distinct_sequences() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let home = TempDir::new().expect("temp home");
    let harness = Arc::new(Harness::open(home.path()).expect("open harness"));
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for n in 0..2 {
        let harness = Arc::clone(&harness);
        let barrier = Arc::clone(&barrier);
        let conversation_id = conversation.id.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            harness
                .session()
                .save(Save::new(&conversation_id).entry("user", json!({"n": n})))
                .expect("save")
                .seq
        }));
    }

    let mut sequences: Vec<i64> = handles
        .into_iter()
        .map(|handle| handle.join().expect("save thread"))
        .collect();
    sequences.sort_unstable();
    assert_eq!(sequences, vec![1, 2]);
    assert_eq!(
        harness
            .session()
            .history(&conversation.id)
            .expect("history")
            .len(),
        2
    );
}

#[test]
fn a_save_to_an_unknown_conversation_is_rejected() {
    let (_home, harness) = harness();
    let error = harness
        .session()
        .save(Save::new("missing").entry("user", json!({})))
        .expect_err("unknown conversation must fail");
    assert!(matches!(
        error,
        hearth_core::CoreError::ConversationNotFound(_)
    ));
}

#[test]
fn creating_an_existing_document_without_a_version_is_rejected() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    session
        .save(Save::new(&conversation.id).document("agent", None, json!({"v": 1})))
        .expect("create document");

    let error = session
        .save(Save::new(&conversation.id).document("agent", None, json!({"v": 2})))
        .expect_err("duplicate create must fail");
    assert!(matches!(
        error,
        hearth_core::CoreError::DocumentVersion {
            expected: None,
            found: Some(1),
            ..
        }
    ));
    assert_eq!(
        session
            .document(&conversation.id, "agent")
            .expect("read document")
            .expect("document exists")
            .version,
        1
    );
}

#[test]
fn a_fork_at_an_entry_from_another_conversation_is_rejected() {
    let (_home, harness) = harness();
    let session = harness.session();
    let first = session
        .create_conversation("First", "local", "local:1")
        .expect("first conversation");
    let second = session
        .create_conversation("Second", "local", "local:2")
        .expect("second conversation");
    let commit = session
        .save(Save::new(&first.id).entry("user", json!({"n": 1})))
        .expect("save");
    let entry_id = &commit.entry_ids[0];

    let error = session
        .fork(&second.id, entry_id)
        .expect_err("foreign entry must fail");
    assert!(matches!(error, hearth_core::CoreError::EntryNotFound(..)));
}

#[test]
fn settling_an_unknown_submission_is_rejected() {
    let (_home, harness) = harness();
    let error = harness
        .session()
        .settle("missing", hearth_core::SubmissionStatus::Done, None)
        .expect_err("unknown submission must fail");
    assert!(matches!(
        error,
        hearth_core::CoreError::SubmissionNotFound(_)
    ));
}

#[test]
fn settling_with_a_non_terminal_status_is_rejected() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let submitted = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
        .expect("submit");

    let error = session
        .settle(
            &submitted.submission.id,
            hearth_core::SubmissionStatus::Queued,
            None,
        )
        .expect_err("non-terminal status must fail");
    assert!(matches!(error, hearth_core::CoreError::Invalid(_)));

    let after = session
        .submission_by_id(&submitted.submission.id)
        .expect("read submission")
        .expect("submission exists");
    assert_eq!(after.status, hearth_core::SubmissionStatus::Queued);
    assert_eq!(after.settled_at, None);
}

#[test]
fn a_save_cannot_settle_a_submission_from_another_conversation() {
    let (_home, harness) = harness();
    let session = harness.session();
    let first = session
        .create_conversation("First", "local", "local:1")
        .expect("first conversation");
    let second = session
        .create_conversation("Second", "local", "local:2")
        .expect("second conversation");
    let submitted = session
        .submit(&first.id, "req-1", "input", json!({"text": "hello"}))
        .expect("submit");

    let error = session
        .save(Save::new(&second.id).settle(
            &submitted.submission.id,
            hearth_core::SubmissionStatus::Done,
            None,
        ))
        .expect_err("cross-conversation settle must fail");
    assert!(matches!(error, hearth_core::CoreError::Invalid(_)));

    let after = session
        .submission_by_id(&submitted.submission.id)
        .expect("read submission")
        .expect("submission exists");
    assert_eq!(after.status, hearth_core::SubmissionStatus::Queued);
    assert_eq!(after.settled_at, None);
}

#[test]
fn the_clock_and_ids_are_injected_and_saves_touch_the_conversation() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    struct TestClock(AtomicI64);
    impl hearth_core::Clock for TestClock {
        fn now_micros(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    struct TestIds(AtomicUsize);
    impl hearth_core::IdGen for TestIds {
        fn new_id(&self) -> String {
            let next = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            format!("id-{next}")
        }
    }

    let home = TempDir::new().expect("temp home");
    let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
    let ids = Arc::new(TestIds(AtomicUsize::new(0)));
    let harness = Harness::open_with(home.path(), clock.clone(), ids).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    assert_eq!(conversation.id, "id-1");
    assert_eq!(conversation.created_at, 1_000);
    assert_eq!(conversation.updated_at, 1_000);

    clock.0.store(2_000, Ordering::SeqCst);
    let commit = session
        .save(Save::new(&conversation.id).entry("user", json!({"n": 1})))
        .expect("save");
    assert_eq!(commit.entry_ids, vec!["id-2".to_owned()]);
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history[0].id, "id-2");
    assert_eq!(history[0].created_at, 2_000);
    let updated = session
        .conversation(&conversation.id)
        .expect("read conversation")
        .expect("conversation exists");
    assert_eq!(updated.updated_at, 2_000);
}

#[test]
fn a_settlement_cannot_use_an_answer_entry_from_another_conversation() {
    let (_home, harness) = harness();
    let session = harness.session();
    let first = session
        .create_conversation("First", "local", "local:1")
        .expect("create first conversation");
    let second = session
        .create_conversation("Second", "local", "local:2")
        .expect("create second conversation");

    let foreign = session
        .save(Save::new(&second.id).entry("assistant", json!({"text": "other"})))
        .expect("save foreign entry");
    let submission = session
        .submit(&first.id, "req-1", "input", json!({"text": "hello"}))
        .expect("submit");
    assert!(submission.created);

    let error = session
        .settle(
            &submission.submission.id,
            SubmissionStatus::Done,
            Some(&foreign.entry_ids[0]),
        )
        .expect_err("a foreign answer entry must be rejected");
    assert!(matches!(error, CoreError::EntryNotFound(..)));

    let after = session
        .submission_by_id(&submission.submission.id)
        .expect("read submission")
        .expect("submission exists");
    assert!(after.settled_at.is_none());
    assert_eq!(after.status, SubmissionStatus::Queued);
}

#[test]
fn a_fork_copies_the_current_documents() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    session
        .save(Save::new(&conversation.id).document("agent", None, json!({"model": "one"})))
        .expect("create document");
    session
        .save(Save::new(&conversation.id).document("agent", Some(1), json!({"model": "two"})))
        .expect("update document");
    session
        .save(Save::new(&conversation.id).entry("user", json!({"text": "fork here"})))
        .expect("save entry");
    let entry = session
        .history(&conversation.id)
        .expect("history")
        .pop()
        .expect("entry");

    let fork = session
        .fork(&conversation.id, &entry.id)
        .expect("fork conversation");

    let document = session
        .document(&fork.id, "agent")
        .expect("read fork document")
        .expect("fork document exists");
    assert_eq!(document.version, 2);
    assert_eq!(document.json["model"], "two");
    let source = session
        .document(&conversation.id, "agent")
        .expect("read source document")
        .expect("source document exists");
    assert_eq!(source.version, 2);
}

#[test]
fn beginning_a_submission_marks_it_running_once() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let submitted = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
        .expect("submit");
    assert_eq!(submitted.submission.status, SubmissionStatus::Queued);

    let running = session
        .begin_submission(&submitted.submission.id)
        .expect("begin submission");
    assert_eq!(running.status, SubmissionStatus::Running);
    let commit = session
        .latest_commit()
        .expect("latest commit")
        .expect("a commit exists");
    assert_eq!(commit.kind, "submission.running");
    let seq = commit.seq;

    let again = session
        .begin_submission(&submitted.submission.id)
        .expect("begin again");
    assert_eq!(again.status, SubmissionStatus::Running);
    let after = session
        .latest_commit()
        .expect("latest commit")
        .expect("a commit exists");
    assert_eq!(after.seq, seq, "an idempotent begin added a commit");
}

#[test]
fn beginning_a_settled_submission_is_rejected() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let submitted = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
        .expect("submit");
    session
        .settle(&submitted.submission.id, SubmissionStatus::Done, None)
        .expect("settle");

    let error = session
        .begin_submission(&submitted.submission.id)
        .expect_err("a settled submission cannot run");
    assert!(matches!(error, CoreError::AlreadySettled(..)));
}

#[test]
fn settling_with_an_error_records_the_reason() {
    use hearth_core::SubmissionStatus;

    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let submission = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "hello"}))
        .expect("submit");

    let settled = session
        .settle_with_error(
            &submission.submission.id,
            SubmissionStatus::Unanswered,
            None,
            Some(json!({"reason": "turn_limit"})),
        )
        .expect("settle");

    assert_eq!(settled.status, SubmissionStatus::Unanswered);
    assert_eq!(settled.error, Some(json!({"reason": "turn_limit"})));
}

#[test]
fn queued_submissions_returns_only_queued_in_order() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let other = session
        .create_conversation("Other", "local", "local:2")
        .expect("create conversation");

    let first = session
        .submit(&conversation.id, "req-1", "input", json!({"text": "one"}))
        .expect("submit first")
        .submission;
    let second = session
        .submit(&conversation.id, "req-2", "input", json!({"text": "two"}))
        .expect("submit second")
        .submission;
    let third = session
        .submit(&conversation.id, "req-3", "input", json!({"text": "three"}))
        .expect("submit third")
        .submission;
    session
        .submit(&other.id, "req-4", "input", json!({"text": "other"}))
        .expect("submit other");
    session
        .begin_submission(&first.id)
        .expect("begin first submission");

    let queued = session
        .queued_submissions(&conversation.id)
        .expect("queued submissions");
    assert_eq!(
        queued
            .iter()
            .map(|submission| submission.id.as_str())
            .collect::<Vec<_>>(),
        vec![second.id.as_str(), third.id.as_str()]
    );
}
