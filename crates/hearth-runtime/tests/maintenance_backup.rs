use std::fs;

use hearth_core::{ClaimedTask, Harness, StepOutput};
use hearth_runtime::TaskHandler;
use hearth_runtime::maintenance::{
    BACKUP_KIND, BackupHandler, MEMORY_CONSOLIDATE_KIND, SKILL_REVIEW_KIND,
    ensure_maintenance_tasks,
};
use hearth_storage::open;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn ensure_creates_the_backup_and_consolidation_tasks() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    ensure_maintenance_tasks(session).expect("ensure maintenance");
    assert!(session.has_open_task(BACKUP_KIND).expect("backup task"));
    assert!(
        session
            .has_open_task(MEMORY_CONSOLIDATE_KIND)
            .expect("memory task")
    );
    assert!(
        session
            .has_open_task(SKILL_REVIEW_KIND)
            .expect("skill task")
    );
}

#[test]
fn the_backup_handler_writes_a_snapshot_and_sleeps() {
    let home = TempDir::new().expect("temp home");
    open(&home.path().join("hearth.db")).expect("open database");
    let task = ClaimedTask {
        id: "backup-task".to_owned(),
        kind: BACKUP_KIND.to_owned(),
        phase: "run".to_owned(),
        conversation_id: None,
        attempt: 1,
        input: json!({}),
        checkpoint: None,
    };
    let output = BackupHandler::new(home.path())
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
}
