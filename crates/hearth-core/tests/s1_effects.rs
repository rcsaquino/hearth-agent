use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use hearth_core::{Clock, EffectIntent, EffectStatus, Harness, IdGen, Save};
use serde_json::json;
use tempfile::TempDir;

struct TestClock(Arc<AtomicI64>);

impl Clock for TestClock {
    fn now_micros(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct TestIds(Arc<AtomicU64>);

impl IdGen for TestIds {
    fn new_id(&self) -> String {
        format!("id-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let clock = Arc::new(TestClock(Arc::new(AtomicI64::new(1_000_000))));
    let ids = Arc::new(TestIds(Arc::new(AtomicU64::new(0))));
    let harness = Harness::open_with(home.path(), clock, ids).expect("open harness");
    (home, harness)
}

fn conversation(session: &hearth_core::Session) -> String {
    session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation")
        .id
}

#[test]
fn a_save_records_an_effect_intent() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation_id = conversation(session);

    let intent = EffectIntent::new(
        "file_write",
        "write-1",
        json!({"path": "notes/a.txt", "content": "hello"}),
    );
    let commit = session
        .save(Save::new(&conversation_id).effect(intent))
        .expect("save effect intent");

    assert_eq!(commit.effect_ids, vec!["id-2".to_owned()]);
    let effect = session
        .effect(&commit.effect_ids[0])
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.kind, "file_write");
    assert_eq!(effect.key, "write-1");
    assert_eq!(effect.status, EffectStatus::Intent);
    assert_eq!(effect.payload["path"], "notes/a.txt");
    assert_eq!(effect.attempts, 0);
    assert_eq!(effect.task_id, None);
    assert_eq!(effect.settled_at, None);
}

#[test]
fn a_duplicate_effect_key_reuses_the_existing_effect() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation_id = conversation(session);

    let first = session
        .save(
            Save::new(&conversation_id)
                .effect(EffectIntent::new("file_write", "write-1", json!({"n": 1}))),
        )
        .expect("first save");
    let second = session
        .save(
            Save::new(&conversation_id)
                .effect(EffectIntent::new("file_write", "write-1", json!({"n": 2}))),
        )
        .expect("second save");

    assert_eq!(first.effect_ids, vec!["id-2".to_owned()]);
    assert_eq!(second.effect_ids, vec!["id-2".to_owned()]);
    let effect = session
        .effect("id-2")
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.payload["n"], 1, "the first intent wins");
    assert_eq!(session.effect("id-3").expect("read missing"), None);
}

#[test]
fn beginning_an_effect_marks_it_started_and_counts_an_attempt() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation_id = conversation(session);
    let commit = session
        .save(
            Save::new(&conversation_id)
                .effect(EffectIntent::new("file_write", "write-1", json!({"path": "a"}))),
        )
        .expect("save");
    let id = commit.effect_ids[0].clone();

    session.begin_effect(&id).expect("begin effect");

    let effect = session
        .effect(&id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Started);
    assert_eq!(effect.attempts, 1);
    assert_eq!(effect.settled_at, None);
}

#[test]
fn finishing_a_started_effect_settles_it_as_done() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation_id = conversation(session);
    let commit = session
        .save(
            Save::new(&conversation_id)
                .effect(EffectIntent::new("file_write", "write-1", json!({"path": "a"}))),
        )
        .expect("save");
    let id = commit.effect_ids[0].clone();
    session.begin_effect(&id).expect("begin effect");

    session
        .finish_effect(&id, json!({"bytes": 5}))
        .expect("finish effect");

    let effect = session
        .effect(&id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Done);
    assert_eq!(effect.result.expect("result")["bytes"], 5);
    assert!(effect.settled_at.is_some());
}

#[test]
fn a_retryable_effect_failure_returns_it_to_intent() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation_id = conversation(session);
    let commit = session
        .save(
            Save::new(&conversation_id)
                .effect(EffectIntent::new("file_write", "write-1", json!({"path": "a"}))),
        )
        .expect("save");
    let id = commit.effect_ids[0].clone();
    session.begin_effect(&id).expect("begin effect");

    session
        .fail_effect(&id, json!({"message": "temporary"}), true)
        .expect("record retryable failure");

    let effect = session
        .effect(&id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Intent);
    assert_eq!(effect.result.expect("result")["message"], "temporary");
    assert_eq!(effect.settled_at, None);
    assert_eq!(effect.attempts, 1);
}

#[test]
fn a_non_retryable_effect_failure_settles_it_as_failed() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation_id = conversation(session);
    let commit = session
        .save(
            Save::new(&conversation_id)
                .effect(EffectIntent::new("file_write", "write-1", json!({"path": "a"}))),
        )
        .expect("save");
    let id = commit.effect_ids[0].clone();
    session.begin_effect(&id).expect("begin effect");

    session
        .fail_effect(&id, json!({"message": "permanent"}), false)
        .expect("record terminal failure");

    let effect = session
        .effect(&id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Failed);
    assert_eq!(effect.result.expect("result")["message"], "permanent");
    assert!(effect.settled_at.is_some());
}
