use hearth_core::{EffectIntent, EffectStatus, Harness, Save};
use hearth_runtime::EffectWorker;
use serde_json::json;
use tempfile::TempDir;

fn setup() -> (TempDir, Harness, String) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("conversation")
        .id;
    (home, harness, conversation)
}

#[test]
fn a_pending_telegram_send_is_left_alone() {
    let (home, harness, conversation) = setup();
    let session = harness.session();
    let commit = session
        .save(Save::new(&conversation).effect(EffectIntent::new(
            "telegram.send",
            "tg:1",
            json!({"chat_id": 42, "markdown": "hello"}),
        )))
        .expect("save effect");
    let id = commit.effect_ids[0].clone();
    let worker = EffectWorker::new(session.clone(), home.path().join("workspace"));

    let processed = worker.run_once().expect("run effects");

    assert_eq!(processed, 0, "telegram sends are not owned by this worker");
    let effect = session.effect(&id).expect("read effect").expect("exists");
    assert_eq!(effect.status, EffectStatus::Intent);
}

#[test]
fn a_started_telegram_send_becomes_unknown_on_recovery() {
    let (home, harness, conversation) = setup();
    let session = harness.session();
    let commit = session
        .save(Save::new(&conversation).effect(EffectIntent::new(
            "telegram.send",
            "tg:2",
            json!({"chat_id": 42, "markdown": "hello"}),
        )))
        .expect("save effect");
    let id = commit.effect_ids[0].clone();
    session.begin_effect(&id).expect("begin effect");
    let worker = EffectWorker::new(session.clone(), home.path().join("workspace"));

    worker.recover_started().expect("recover");

    let effect = session.effect(&id).expect("read effect").expect("exists");
    assert_eq!(effect.status, EffectStatus::Unknown);
    assert_eq!(
        effect.result.expect("result")["error"],
        "delivery interrupted"
    );
}
