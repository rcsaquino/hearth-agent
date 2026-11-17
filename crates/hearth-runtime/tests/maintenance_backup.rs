use std::fs;

use hearth_core::{ClaimedTask, Harness, StepOutput, Zone};
use hearth_runtime::TaskHandler;
use hearth_runtime::maintenance::{
    BACKUP_KIND, BackupHandler, MEMORY_CONSOLIDATION_KIND, MemoryConsolidationHandler,
    SKILL_REVIEW_KIND, ensure_maintenance_tasks,
};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn ensure_creates_the_backup_and_consolidation_tasks() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    ensure_maintenance_tasks(session, &Zone::utc(), 30, 25).expect("ensure maintenance");
    assert!(session.has_open_task(BACKUP_KIND).expect("backup task"));
    assert!(
        session
            .has_open_task(SKILL_REVIEW_KIND)
            .expect("skill task")
    );
    assert!(
        session
            .has_open_task(MEMORY_CONSOLIDATION_KIND)
            .expect("memory task")
    );
}

#[test]
fn the_backup_handler_writes_a_snapshot_and_sleeps() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let hash = session
        .save_prompt_blob(r#"{"identity":"SOUL TEXT"}"#)
        .expect("prompt blob");
    let old = session.now_micros() - hearth_core::PROMPT_BLOB_RETENTION_MICROS - 1;
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
    let task = ClaimedTask {
        id: "backup-task".to_owned(),
        kind: BACKUP_KIND.to_owned(),
        phase: "run".to_owned(),
        conversation_id: None,
        attempt: 1,
        input: json!({}),
        checkpoint: None,
    };
    let output = BackupHandler::new(home.path().to_path_buf(), session.clone())
        .step(&task)
        .expect("run backup handler");
    assert!(matches!(output, StepOutput::Sleep { .. }));

    let backups: Vec<String> = fs::read_dir(home.path().join("backups"))
        .expect("read backups")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| name.starts_with("hearth-") && name.ends_with(".db"))
        .collect();
    assert_eq!(backups.len(), 1, "backups: {backups:?}");
    assert!(
        session.latest_prompt().expect("latest prompt").is_none(),
        "the nightly pass must prune a blob past its retention window"
    );
}

#[test]
fn the_consolidation_handler_repairs_publishes_and_sleeps() {
    use hearth_memory::{MemoryService, fingerprint};
    use hearth_runtime::EventBus;
    use std::sync::Arc;

    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let now = session.now_micros();
    let content = "the user prefers dark mode";
    let digest = fingerprint(content);
    session
        .storage()
        .write(move |writer| {
            for (id, kind) in [("m_dup1", "fact"), ("m_dup2", "preference")] {
                writer.execute(
                    "INSERT INTO memories (id, kind, content, subject, terms, fingerprint, pinned, \
                     source_kind, source_entry, valid_from, valid_to, created_at, updated_at, revision) \
                     VALUES (?, ?, ?, 'user', '', ?, 0, 'user', NULL, ?, NULL, ?, ?, 1)",
                    hearth_storage::rusqlite::params![id, kind, content, digest, now, now, now],
                )?;
            }
            Ok::<_, hearth_memory::MemoryError>(())
        })
        .expect("seed duplicates");

    let bus = Arc::new(EventBus::new(Vec::new()));
    let receiver = bus.subscribe();
    let memory = Arc::new(MemoryService::new(session.clone()));
    let handler = MemoryConsolidationHandler::new(
        home.path().to_path_buf(),
        Arc::clone(&memory),
        Some(Arc::clone(&bus)),
    );
    let task = ClaimedTask {
        id: "consolidate-task".to_owned(),
        kind: MEMORY_CONSOLIDATION_KIND.to_owned(),
        phase: "run".to_owned(),
        conversation_id: None,
        attempt: 1,
        input: json!({}),
        checkpoint: None,
    };
    let output = handler.step(&task).expect("run the pass");
    assert!(matches!(output, StepOutput::Sleep { .. }), "{output:?}");
    assert_eq!(memory.count().expect("count"), 1);
    assert!(
        session
            .meta_value("last_consolidation")
            .expect("stamp lookup")
            .is_some(),
        "the pass must record when it ran"
    );
    let line = receiver.recv().expect("event");
    let event: serde_json::Value = serde_json::from_str(&line).expect("json event");
    assert_eq!(event["kind"], "memory.consolidated");
    assert_eq!(event["data"]["canonical_closed"], 1, "{event}");
}
