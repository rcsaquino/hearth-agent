use std::fs;

use hearth_storage::{SCHEMA_VERSION, open, open_read_only, snapshot, verify};
use tempfile::TempDir;

fn database() -> (TempDir, std::path::PathBuf) {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("hearth.db");
    (directory, path)
}

#[test]
fn a_snapshot_copies_the_live_database() {
    let (directory, source) = database();
    {
        let connection = open(&source).expect("open database");
        connection
            .execute(
                "INSERT INTO meta (key, value) VALUES ('snapshot-test', 'kept')",
                [],
            )
            .expect("insert a row");
    }
    let destination = directory.path().join("snapshot.db");
    snapshot(&source, &destination).expect("snapshot");
    assert_eq!(
        verify(&destination).expect("verify snapshot"),
        SCHEMA_VERSION
    );
    let connection = open_read_only(&destination).expect("open snapshot");
    let value: String = connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'snapshot-test'",
            [],
            |row| row.get(0),
        )
        .expect("read snapshot row");
    assert_eq!(value, "kept");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&destination)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn a_missing_database_is_not_snapshotted() {
    let (directory, source) = database();
    let destination = directory.path().join("snapshot.db");
    assert!(snapshot(&source, &destination).is_err());
    assert!(!destination.exists());
}

#[test]
fn verification_rejects_a_corrupt_file() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("bad.db");
    fs::write(&path, b"this is not a database").expect("write bad file");
    assert!(verify(&path).is_err());
}

#[test]
fn a_snapshot_replaces_an_existing_destination_atomically() {
    let (directory, source) = database();
    {
        let connection = open(&source).expect("open database");
        connection
            .execute("INSERT INTO meta (key, value) VALUES ('a', 'one')", [])
            .expect("insert");
    }
    let destination = directory.path().join("snapshot.db");
    fs::write(&destination, b"old").expect("old destination");
    snapshot(&source, &destination).expect("snapshot");
    assert_eq!(
        verify(&destination).expect("verify replacement"),
        SCHEMA_VERSION
    );
}

#[test]
fn a_snapshot_is_private_while_it_is_copied() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let (directory, source) = database();
    {
        let connection = open(&source).expect("open database");
        for index in 0..20_000 {
            connection
                .execute(
                    "INSERT INTO meta (key, value) VALUES (?, ?)",
                    rusqlite::params![format!("row-{index}"), "x".repeat(200)],
                )
                .expect("insert");
        }
    }
    let destination = directory.path().join("snapshot.db");
    let done = Arc::new(AtomicBool::new(false));
    let watching = Arc::clone(&done);
    let folder = directory.path().to_path_buf();
    let watcher = std::thread::spawn(move || {
        let mut exposed = None;
        while !watching.load(Ordering::Relaxed) {
            let Ok(entries) = fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let mode = metadata.permissions().mode() & 0o077;
                if mode != 0 && metadata.is_file() {
                    exposed = Some((entry.file_name(), mode));
                }
            }
        }
        exposed
    });
    snapshot(&source, &destination).expect("snapshot");
    done.store(true, Ordering::Relaxed);
    let exposed = watcher.join().expect("watcher");
    assert_eq!(
        exposed, None,
        "a snapshot file was readable by group or other while it was copied"
    );
}
