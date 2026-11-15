use hearth_core::{Harness, NewTask, NewUsage};
use hearth_model::Usage;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn recording_usage_keeps_every_field() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let task = session
        .create_task(
            NewTask::new("test.usage", 1, "start", json!({})).conversation(&conversation.id),
        )
        .expect("create task");

    let recorded = session
        .record_usage(NewUsage {
            task_id: Some(task.id.clone()),
            conversation_id: Some(conversation.id.clone()),
            provider: "openai-compatible".to_owned(),
            model: "test-model".to_owned(),
            kind: "chat".to_owned(),
            usage: Usage {
                input_tokens: 100,
                cache_read_tokens: 20,
                cache_write_tokens: 5,
                output_tokens: 30,
                reasoning_tokens: 7,
            },
            latency_ms: 250,
        })
        .expect("record usage");

    assert!(!recorded.id.is_empty());
    assert!(recorded.ts > 0);
    assert_eq!(recorded.task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(recorded.input_tokens, 100);
    assert_eq!(recorded.cache_read_tokens, 20);
    assert_eq!(recorded.cache_write_tokens, 5);
    assert_eq!(recorded.output_tokens, 30);
    assert_eq!(recorded.reasoning_tokens, 7);
    assert_eq!(recorded.latency_ms, 250);

    let read = session.usage_for_task(&task.id).expect("read usage");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0], recorded);
}

#[test]
fn the_latest_usage_row_is_available() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let task = session
        .create_task(
            NewTask::new("test.usage", 1, "start", json!({})).conversation(&conversation.id),
        )
        .expect("create task");
    let recorded = session
        .record_usage(NewUsage {
            task_id: Some(task.id.clone()),
            conversation_id: Some(conversation.id.clone()),
            provider: "openai-compatible".to_owned(),
            model: "test-model".to_owned(),
            kind: "chat".to_owned(),
            usage: Usage::default(),
            latency_ms: 1,
        })
        .expect("record usage");

    let latest = session
        .latest_usage()
        .expect("latest usage")
        .expect("a usage row");
    assert_eq!(latest.id, recorded.id);
}

#[test]
fn the_cache_hit_rate_is_cache_reads_over_prompt_tokens() {
    assert_eq!(hearth_core::cache_hit_rate(100, 80), 0.8);
    assert_eq!(hearth_core::cache_hit_rate(0, 0), 0.0);
    assert_eq!(hearth_core::cache_hit_rate(100, 0), 0.0);
    assert_eq!(hearth_core::cache_hit_rate(100, 140), 1.0);
}

#[test]
fn an_old_prompt_blob_is_pruned_and_its_entry_survives() {
    use hearth_core::{Harness, NewEntry, PROMPT_BLOB_RETENTION_MICROS, Save};
    use serde_json::json;
    use tempfile::TempDir;

    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Prune", "local", "local:prune")
        .expect("conversation");
    let hash = session
        .save_prompt_blob(r#"{"identity":"SOUL TEXT"}"#)
        .expect("prompt blob");
    session
        .save(Save::new(&conversation.id).entry_with(NewEntry {
            kind: "assistant".to_owned(),
            content: json!({"text": "the answer"}),
            source: None,
            prompt_hash: Some(hash.clone()),
        }))
        .expect("entry");

    let old = session.now_micros() - PROMPT_BLOB_RETENTION_MICROS - 1;
    let write_hash = hash.clone();
    session
        .storage()
        .write(move |writer| {
            writer.execute(
                "UPDATE prompt_blobs SET created_at = ? WHERE hash = ?",
                (old, write_hash.as_str()),
            )?;
            Ok::<_, hearth_storage::StorageError>(())
        })
        .expect("age the blob");

    let removed = session
        .prune_prompt_blobs(session.now_micros() - PROMPT_BLOB_RETENTION_MICROS)
        .expect("prune");
    assert_eq!(removed, 1, "the aged blob must be removed");
    assert!(
        session.latest_prompt().expect("latest").is_none(),
        "the pruned blob must be gone"
    );
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 1);
    assert!(history[0].prompt_hash.is_none());
    assert_eq!(history[0].content["text"], "the answer");
}
