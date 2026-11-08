use std::path::PathBuf;

use hearth_storage::Storage;
use tempfile::TempDir;

fn database() -> (TempDir, PathBuf) {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("hearth.db");
    (directory, path)
}

#[test]
fn a_fresh_database_reaches_schema_version_one() {
    let (_directory, path) = database();
    let storage = Storage::open(&path).expect("open storage");
    assert_eq!(storage.schema_version().expect("schema version"), 1);
}

#[test]
fn reopening_the_database_is_idempotent() {
    let (_directory, path) = database();
    {
        let storage = Storage::open(&path).expect("open storage");
        assert_eq!(storage.schema_version().expect("schema version"), 1);
    }
    let storage = Storage::open(&path).expect("reopen storage");
    assert_eq!(storage.schema_version().expect("schema version"), 1);
}

#[test]
fn a_future_schema_version_is_rejected() {
    let (_directory, path) = database();
    {
        let connection = rusqlite::Connection::open(&path).expect("create database");
        connection
            .pragma_update(None, "user_version", 99)
            .expect("set future version");
    }

    let error = match Storage::open(&path) {
        Ok(_) => panic!("future version must fail"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        hearth_storage::StorageError::SchemaVersion {
            found: 99,
            expected: 1
        }
    ));
}

#[test]
fn a_failed_write_leaves_no_commit_and_no_sequence_gap() {
    let (_directory, path) = database();
    let storage = Storage::open(&path).expect("open storage");

    let failed: Result<i64, hearth_storage::StorageError> = storage.write(|writer| {
        writer.execute(
            "INSERT INTO commits (seq, created_at, conversation_id, kind) VALUES (1, 0, NULL, 'test')",
            [],
        )?;
        Err(hearth_storage::StorageError::Closed)
    });
    assert!(failed.is_err());

    let count: i64 = storage
        .read(|reader| reader.query_row("SELECT COUNT(*) FROM commits", [], |row| row.get(0)))
        .expect("count commits");
    assert_eq!(count, 0);

    let next: i64 = storage
        .write(|writer| -> Result<i64, hearth_storage::StorageError> {
            let seq: i64 = writer.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM commits",
                [],
                |row| row.get(0),
            )?;
            writer.execute(
                "INSERT INTO commits (seq, created_at, conversation_id, kind) VALUES (?, 0, NULL, 'test')",
                (seq,),
            )?;
            Ok(seq)
        })
        .expect("write after failure");
    assert_eq!(next, 1);
}

#[test]
fn sequential_writes_get_increasing_sequences() {
    let (_directory, path) = database();
    let storage = Storage::open(&path).expect("open storage");

    let first = append_commit(&storage, "first");
    let second = append_commit(&storage, "second");

    assert_eq!(first, 1);
    assert_eq!(second, 2);
}

fn append_commit(storage: &Storage, kind: &str) -> i64 {
    let kind = kind.to_owned();
    storage
        .write(move |writer| -> Result<i64, hearth_storage::StorageError> {
            let seq: i64 = writer.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM commits",
                [],
                |row| row.get(0),
            )?;
            writer.execute(
                "INSERT INTO commits (seq, created_at, conversation_id, kind) VALUES (?, 0, NULL, ?)",
                (seq, kind),
            )?;
            Ok(seq)
        })
        .expect("append commit")
}

#[test]
fn concurrent_writes_are_serialized() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let (_directory, path) = database();
    let storage = Arc::new(Storage::open(&path).expect("open storage"));
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for kind in ["left", "right"] {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        let kind = kind.to_owned();
        handles.push(thread::spawn(move || {
            barrier.wait();
            append_commit(&storage, &kind)
        }));
    }

    let mut sequences: Vec<i64> = handles
        .into_iter()
        .map(|handle| handle.join().expect("writer thread"))
        .collect();
    sequences.sort_unstable();
    assert_eq!(sequences, vec![1, 2]);

    let count: i64 = storage
        .read(|reader| reader.query_row("SELECT COUNT(*) FROM commits", [], |row| row.get(0)))
        .expect("count commits");
    assert_eq!(count, 2);
}

#[test]
fn a_read_does_not_see_an_uncommitted_write() {
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    let (_directory, path) = database();
    let storage = Arc::new(Storage::open(&path).expect("open storage"));
    let (inserted_tx, inserted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let writer = {
        let storage = Arc::clone(&storage);
        thread::spawn(move || {
            storage.write(move |writer| -> Result<(), hearth_storage::StorageError> {
                writer.execute(
                    "INSERT INTO commits (seq, created_at, conversation_id, kind) VALUES (1, 0, NULL, 'pending')",
                    [],
                )?;
                inserted_tx.send(()).expect("signal inserted row");
                release_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("release writer");
                Ok(())
            })
        })
    };

    inserted_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("row inserted");
    let before: i64 = storage
        .read(|reader| reader.query_row("SELECT COUNT(*) FROM commits", [], |row| row.get(0)))
        .expect("read before commit");
    assert_eq!(before, 0);

    release_tx.send(()).expect("release writer");
    writer.join().expect("join writer").expect("write");
    let after: i64 = storage
        .read(|reader| reader.query_row("SELECT COUNT(*) FROM commits", [], |row| row.get(0)))
        .expect("read after commit");
    assert_eq!(after, 1);
}

#[test]
fn the_read_pool_serves_four_concurrent_reads() {
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    struct Gate {
        count: Mutex<usize>,
        ready: Condvar,
    }

    impl Gate {
        fn arrive(&self) -> bool {
            let mut count = self.count.lock().expect("gate lock");
            *count += 1;
            self.ready.notify_all();
            let deadline = Instant::now() + Duration::from_secs(2);
            while *count < 4 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return false;
                }
                let (guard, timeout) = self
                    .ready
                    .wait_timeout(count, remaining)
                    .expect("gate wait");
                count = guard;
                if timeout.timed_out() && *count < 4 {
                    return false;
                }
            }
            true
        }
    }

    let (_directory, path) = database();
    let storage = Arc::new(Storage::open(&path).expect("open storage"));
    let gate = Arc::new(Gate {
        count: Mutex::new(0),
        ready: Condvar::new(),
    });

    let mut handles = Vec::new();
    for _ in 0..4 {
        let storage = Arc::clone(&storage);
        let gate = Arc::clone(&gate);
        handles.push(thread::spawn(move || {
            storage.read(|_reader| {
                if gate.arrive() {
                    Ok(())
                } else {
                    Err(hearth_storage::StorageError::Closed)
                }
            })
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("reader thread")
            .expect("four readers arrived");
    }
}

#[test]
fn a_fifth_concurrent_read_waits_for_a_free_connection() {
    use std::sync::{Arc, Barrier, Condvar, Mutex, mpsc};
    use std::thread;
    use std::time::Duration;

    let (_directory, path) = database();
    let storage = Arc::new(Storage::open(&path).expect("open storage"));
    let start = Arc::new(Barrier::new(5));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));

    let mut handles = Vec::new();
    for _ in 0..5 {
        let storage = Arc::clone(&storage);
        let start = Arc::clone(&start);
        let entered_tx = entered_tx.clone();
        let done_tx = done_tx.clone();
        let release = Arc::clone(&release);
        handles.push(thread::spawn(move || {
            start.wait();
            let result = storage.read(|_reader| {
                entered_tx.send(()).expect("signal entered read");
                let (lock, ready) = &*release;
                let mut released = lock.lock().expect("release lock");
                while !*released {
                    released = ready.wait(released).expect("release wait");
                }
                Ok(())
            });
            done_tx.send(result).expect("signal read done");
        }));
    }
    drop(entered_tx);
    drop(done_tx);

    for _ in 0..4 {
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("four readers entered");
    }

    match done_rx.recv_timeout(Duration::from_millis(200)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Ok(result) => panic!("a read completed while all four connections were busy: {result:?}"),
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("reader channel closed early"),
    }

    {
        let (lock, ready) = &*release;
        *lock.lock().expect("release lock") = true;
        ready.notify_all();
    }

    let mut completed = 0;
    while completed < 5 {
        let result = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("all five reads completed");
        result.expect("read succeeded");
        completed += 1;
    }

    for handle in handles {
        handle.join().expect("reader thread");
    }
}
