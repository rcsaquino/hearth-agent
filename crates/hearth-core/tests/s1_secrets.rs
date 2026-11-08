use hearth_core::secrets::{contains_secret, redact_secrets, redact_value};
use hearth_core::{
    CoreError, EffectIntent, Harness, MemoryChange, MemoryWrite, NewMemory, NewTask, Save,
};
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

#[test]
fn plain_text_is_unchanged_and_credentials_are_detected() {
    assert_eq!(redact_secrets("hello world"), "hello world");
    assert!(!contains_secret("hello world"));

    for text in [
        "the key is sk-test-not-a-real-key",
        "Bearer abcdefghijkl",
        "token=abcdefghijk",
        "12345678:abcdefghijklmnopqrstuvwx",
        "ghp_abcdefghijklmnop",
    ] {
        assert!(contains_secret(text), "{text} should be detected");
    }

    let redacted = redact_secrets("the key is sk-test-not-a-real-key and more");
    assert!(!redacted.contains("sk-test-not-a-real-key"));
    assert!(redacted.contains("[redacted]"));
    assert!(redacted.contains("and more"));
}

#[test]
fn redact_value_walks_arrays_and_objects() {
    let value = json!({
        "text": "token=abcdefghijk",
        "nested": ["ok", {"inner": "sk-test-not-a-real-key"}],
        "number": 7
    });
    let redacted = redact_value(&value);
    assert!(!redacted.to_string().contains("abcdefghijk"));
    assert!(!redacted.to_string().contains("sk-test-not-a-real-key"));
    assert_eq!(redacted["number"], 7);
}

#[test]
fn the_record_stores_no_credential_shaped_text() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Title sk-test-not-a-real-key", "local", "local:1")
        .expect("conversation");
    assert!(!conversation.title.contains("sk-test"));

    let submitted = session
        .submit(
            &conversation.id,
            "req-secret-1",
            "input",
            json!({"text": "my key is sk-test-not-a-real-key"}),
        )
        .expect("submit");
    assert!(!submitted.submission.content.to_string().contains("sk-test"));

    let commit = session
        .save(
            Save::new(&conversation.id)
                .entry("assistant", json!({"text": "token=abcdefghijk"}))
                .document("agent", None, json!({"note": "Bearer abcdefghijkl"}))
                .effect(EffectIntent::new(
                    "file_write",
                    "secret-effect",
                    json!({"content": "sk-test-not-a-real-key"}),
                )),
        )
        .expect("save");
    let effect_id = commit.effect_ids[0].clone();

    let task = session
        .create_task(
            NewTask::new(
                "helper",
                1,
                "start",
                json!({"brief": "use sk-test-not-a-real-key"}),
            )
            .conversation(&conversation.id),
        )
        .expect("task");
    assert!(!task.input.to_string().contains("sk-test"));

    let history = session.history(&conversation.id).expect("history");
    let record = history
        .iter()
        .map(|entry| entry.content.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!record.contains("sk-test"));
    assert!(!record.contains("abcdefghijk"));

    let document = session
        .document(&conversation.id, "agent")
        .expect("document")
        .expect("document exists");
    assert!(!document.json.to_string().contains("abcdefghijkl"));

    let effect = session
        .effect(&effect_id)
        .expect("effect")
        .expect("effect exists");
    assert!(!effect.payload.to_string().contains("sk-test"));

    let task = session
        .task(&task.id)
        .expect("task read")
        .expect("task exists");
    assert!(!task.input.to_string().contains("sk-test"));

    let blob = session
        .save_prompt_blob("identity sk-test-not-a-real-key")
        .expect("prompt blob");
    let latest = session.latest_prompt().expect("latest").expect("prompt");
    assert_eq!(latest.hash, blob);
    assert!(!latest.text.contains("sk-test"));
}

#[test]
fn the_core_refuses_a_secret_memory_write() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let error = session
        .save(Save::new(&conversation.id).memory(MemoryWrite::new(
            MemoryChange::Add(NewMemory {
                id: "01a1bbc8-27b8-7254-94e2-c8cd745fcc0a".to_owned(),
                kind: "fact".to_owned(),
                content: "the key is sk-test-not-a-real-key".to_owned(),
                tags: vec![],
                fingerprint: "fingerprint-secret".to_owned(),
                importance: 50,
                status: "active".to_owned(),
                valid_from: None,
                valid_to: None,
                supersedes_id: None,
                source_kind: "user".to_owned(),
                source_ref: None,
                standing: "none".to_owned(),
                created_at: 1,
                last_used_at: None,
            }),
            "user",
        )))
        .expect_err("secret memory must be refused");
    assert!(matches!(error, CoreError::Invalid(_)));
}
