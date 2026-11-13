use std::fs::{self, OpenOptions};
use std::path::Path;

use hearth_runtime::backup::{BackupError, backup, prune, restore};
use hearth_storage::{open, open_read_only};
use tempfile::TempDir;
use time::{Date, Duration, Month};

fn home_with_database() -> TempDir {
    let home = TempDir::new().expect("temp home");
    let connection = open(&home.path().join("hearth.db")).expect("open database");
    connection
        .execute("INSERT INTO meta (key, value) VALUES ('kept', 'yes')", [])
        .expect("insert");
    home
}

fn backup_name(date: Date) -> String {
    format!(
        "hearth-{:04}{:02}{:02}.db",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

#[test]
fn backup_and_restore_round_trip() {
    let home = home_with_database();
    let report = backup(home.path()).expect("backup");
    assert!(report.path.is_file());
    assert_eq!(
        hearth_storage::verify(&report.path).expect("verify backup"),
        hearth_storage::SCHEMA_VERSION
    );

    {
        let connection = open(&home.path().join("hearth.db")).expect("open database");
        connection
            .execute("INSERT INTO meta (key, value) VALUES ('after', 'gone')", [])
            .expect("insert after backup");
    }

    let restored = restore(home.path(), &report.path).expect("restore");
    assert!(restored.pre_restore.as_deref().is_some_and(Path::is_file));
    let connection = open_read_only(&home.path().join("hearth.db")).expect("open restored");
    let kept: String = connection
        .query_row("SELECT value FROM meta WHERE key = 'kept'", [], |row| {
            row.get(0)
        })
        .expect("kept row");
    assert_eq!(kept, "yes");
    let after: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE key = 'after'", [], |row| {
            row.get(0)
        })
        .ok();
    assert!(after.is_none(), "the discarded row survived the restore");
}

#[test]
fn backup_and_restore_include_the_memory_stores() {
    let home = home_with_database();
    let memory_dir = home.path().join("memory");
    fs::create_dir_all(&memory_dir).expect("memory directory");
    {
        let memory = hearth_memory::MemoryStore::open(memory_dir.join("long-term.sqlite"))
            .expect("memory store");
        memory
            .add(&hearth_memory::MemoryInput {
                content: "the kept memory".to_owned(),
                ..Default::default()
            })
            .expect("save memory");
    }
    fs::write(
        memory_dir.join("standing.md"),
        "- keep this standing rule\n",
    )
    .expect("standing file");

    let report = backup(home.path()).expect("backup");
    assert!(report.memory.as_deref().is_some_and(Path::is_file));
    assert!(report.standing.as_deref().is_some_and(Path::is_file));

    {
        let memory = hearth_memory::MemoryStore::open(memory_dir.join("long-term.sqlite"))
            .expect("memory store");
        memory
            .add(&hearth_memory::MemoryInput {
                content: "the discarded memory".to_owned(),
                ..Default::default()
            })
            .expect("save memory");
    }
    fs::write(memory_dir.join("standing.md"), "- changed rule\n").expect("standing change");

    let restored = restore(home.path(), &report.path).expect("restore");
    assert!(
        restored
            .memory_restored
            .as_deref()
            .is_some_and(Path::is_file)
    );
    assert!(
        restored
            .standing_restored
            .as_deref()
            .is_some_and(Path::is_file)
    );

    let memory = hearth_memory::MemoryStore::open(memory_dir.join("long-term.sqlite"))
        .expect("reopen memory");
    let kept = memory
        .search(
            "kept",
            hearth_memory::SearchMode::Any,
            10,
            0,
            &hearth_memory::MemorySearchOptions::default(),
        )
        .expect("search kept");
    assert_eq!(kept.results.len(), 1);
    let discarded = memory
        .search(
            "discarded",
            hearth_memory::SearchMode::Any,
            10,
            0,
            &hearth_memory::MemorySearchOptions::default(),
        )
        .expect("search discarded");
    assert!(discarded.results.is_empty());
    let standing = fs::read_to_string(memory_dir.join("standing.md")).expect("standing");
    assert!(standing.contains("keep this standing rule"), "{standing}");
}

#[test]
fn backup_works_while_a_writer_is_open() {
    let home = TempDir::new().expect("temp home");
    let harness = hearth_core::Harness::open(home.path()).expect("open harness");
    harness
        .session()
        .create_conversation("Live", "local", "local:1")
        .expect("conversation");
    let report = backup(home.path()).expect("backup while open");
    assert_eq!(
        hearth_storage::verify(&report.path).expect("verify"),
        hearth_storage::SCHEMA_VERSION
    );
    drop(harness);
}

#[test]
fn restore_refuses_a_running_daemon() {
    let home = home_with_database();
    let report = backup(home.path()).expect("backup");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.path().join("hearth.lock"))
        .expect("open lock");
    lock.try_lock().expect("hold lock");
    let error = restore(home.path(), &report.path).expect_err("restore must refuse");
    assert!(matches!(error, BackupError::Running));
    drop(lock);
    restore(home.path(), &report.path).expect("restore after release");
}

#[test]
fn restore_rejects_a_corrupt_backup() {
    let home = home_with_database();
    let bad = home.path().join("bad.db");
    fs::write(&bad, b"not a database").expect("write bad backup");
    let error = restore(home.path(), &bad).expect_err("restore must reject");
    assert!(matches!(error, BackupError::InvalidBackup(_)));
    let connection = open_read_only(&home.path().join("hearth.db")).expect("database intact");
    let kept: String = connection
        .query_row("SELECT value FROM meta WHERE key = 'kept'", [], |row| {
            row.get(0)
        })
        .expect("kept row");
    assert_eq!(kept, "yes");
}

#[test]
fn backup_and_restore_report_missing_paths() {
    let empty = TempDir::new().expect("empty home");
    let error = backup(empty.path()).expect_err("missing database");
    assert!(matches!(error, BackupError::MissingDatabase(_)));

    let home = home_with_database();
    let error = restore(home.path(), &home.path().join("nope.db")).expect_err("missing backup");
    assert!(matches!(error, BackupError::MissingBackup(_)));
}

#[test]
fn retention_keeps_seven_daily_and_four_weekly() {
    let directory = TempDir::new().expect("backup directory");
    let start = Date::from_calendar_date(2026, Month::October, 10).expect("start date");
    for offset in 0..30 {
        let date = start + Duration::days(offset);
        fs::write(directory.path().join(backup_name(date)), b"snapshot").expect("write backup");
    }
    let today = Date::from_calendar_date(2026, Month::November, 8).expect("today");
    let removed = prune(directory.path(), today).expect("prune");
    assert_eq!(removed.len(), 20);

    let mut remaining: Vec<String> = fs::read_dir(directory.path())
        .expect("read backups")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| name.starts_with("hearth-") && name.ends_with(".db"))
        .collect();
    remaining.sort();
    let expected: Vec<String> = [
        "hearth-20261018.db",
        "hearth-20261025.db",
        "hearth-20261101.db",
        "hearth-20261102.db",
        "hearth-20261103.db",
        "hearth-20261104.db",
        "hearth-20261105.db",
        "hearth-20261106.db",
        "hearth-20261107.db",
        "hearth-20261108.db",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(remaining, expected);
}
