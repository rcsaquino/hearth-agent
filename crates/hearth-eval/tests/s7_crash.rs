use std::path::Path;
use std::process::{Command, Output};

use hearth_core::{Harness, SubmissionStatus, TaskStatus};
use tempfile::TempDir;

fn home() -> TempDir {
    TempDir::new().expect("temp home")
}

fn conversation(home: &Path) -> String {
    let harness = Harness::open(home).expect("open harness");
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    conversation.id
}

fn driver() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crash-driver"))
}

fn run(home: &Path, args: &[&str], crash_at: Option<&str>) -> Output {
    let mut command = driver();
    command.args(args);
    command.current_dir(home);
    if let Some(point) = crash_at {
        command.env("HEARTH_CRASH_AT", point);
    }
    command.output().expect("run crash driver")
}

fn output_text(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_crash_before_commit_leaves_no_entry_and_no_gap() {
    let home = home();
    let home_text = home.path().to_str().expect("utf-8 home");
    let conversation = conversation(home.path());

    let crashed = run(
        home.path(),
        &["save", home_text, conversation.as_str()],
        Some("before_commit"),
    );
    assert!(
        !crashed.status.success(),
        "the driver did not crash: {}",
        output_text(&crashed)
    );

    {
        let harness = Harness::open(home.path()).expect("reopen harness");
        assert!(
            harness
                .session()
                .history(&conversation)
                .expect("history")
                .is_empty(),
            "an entry was committed before the crash point"
        );
        assert!(
            harness
                .session()
                .latest_commit()
                .expect("latest commit")
                .is_none(),
            "a commit survived the crash"
        );
    }

    let retry = run(
        home.path(),
        &["save", home_text, conversation.as_str()],
        None,
    );
    assert!(
        retry.status.success(),
        "the retry failed: {}",
        output_text(&retry)
    );

    let harness = Harness::open(home.path()).expect("reopen harness");
    assert_eq!(
        harness
            .session()
            .history(&conversation)
            .expect("history")
            .len(),
        1
    );
    let commit = harness
        .session()
        .latest_commit()
        .expect("latest commit")
        .expect("a commit exists");
    assert_eq!(commit.seq, 1);
    assert_eq!(commit.kind, "save");
}

#[test]
fn a_crash_after_commit_keeps_a_submission_once() {
    let home = home();
    let home_text = home.path().to_str().expect("utf-8 home");
    let conversation = conversation(home.path());

    let crashed = run(
        home.path(),
        &["submit", home_text, conversation.as_str(), "req-1"],
        Some("after_commit"),
    );
    assert!(
        !crashed.status.success(),
        "the driver did not crash: {}",
        output_text(&crashed)
    );

    {
        let harness = Harness::open(home.path()).expect("reopen harness");
        assert_eq!(
            harness
                .session()
                .history(&conversation)
                .expect("history")
                .len(),
            1
        );
        let submission = harness
            .session()
            .submission_by_request_id("req-1")
            .expect("read submission")
            .expect("submission survived");
        assert_eq!(submission.status, hearth_core::SubmissionStatus::Queued);
        let commit = harness
            .session()
            .latest_commit()
            .expect("latest commit")
            .expect("a commit exists");
        assert_eq!(commit.seq, 1);
        assert_eq!(commit.kind, "save");
    }

    let harness = Harness::open(home.path()).expect("reopen harness");
    let result = harness
        .session()
        .submit(
            &conversation,
            "req-1",
            "input",
            serde_json::json!({"text": "hello"}),
        )
        .expect("repeat submission");
    assert!(!result.created, "the repeat created a second submission");
    assert_eq!(
        harness
            .session()
            .history(&conversation)
            .expect("history")
            .len(),
        1
    );
    let commit = harness
        .session()
        .latest_commit()
        .expect("latest commit")
        .expect("a commit exists");
    assert_eq!(commit.seq, 1, "the repeat advanced the sequence");
}

#[test]
fn a_crash_mid_stream_keeps_the_input_and_retries_once() {
    let home = home();
    let home_text = home.path().to_str().expect("utf-8 home");
    let conversation = conversation(home.path());

    let crashed = run(
        home.path(),
        &[
            "agent",
            home_text,
            conversation.as_str(),
            "req-agent-mid-stream",
        ],
        Some("mid_stream"),
    );
    assert!(
        !crashed.status.success(),
        "the driver did not crash: {}",
        output_text(&crashed)
    );

    {
        let harness = Harness::open(home.path()).expect("reopen harness");
        let session = harness.session();
        let history = session.history(&conversation).expect("history");
        assert_eq!(history.len(), 1, "history was {history:?}");
        assert_eq!(history[0].kind, "user");
        assert!(
            !history.iter().any(|entry| entry.kind == "assistant"),
            "an assistant entry was saved before the crash"
        );
        let submission = session
            .submission_by_request_id("req-agent-mid-stream")
            .expect("submission lookup")
            .expect("submission exists");
        assert_eq!(submission.status, SubmissionStatus::Running);

        let task_id: String = session
            .storage()
            .read(|reader| {
                reader.query_row(
                    "SELECT id FROM tasks WHERE kind = 'generation' ORDER BY created_at DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
            })
            .expect("generation task");
        let task = session
            .task(&task_id)
            .expect("read task")
            .expect("task exists");
        assert_eq!(task.status, TaskStatus::Running);

        let expire_id = task_id.clone();
        session
            .storage()
            .write(move |writer| -> Result<(), hearth_storage::StorageError> {
                writer.execute(
                    "UPDATE tasks SET lease_until = 0 WHERE id = ?",
                    (&expire_id,),
                )?;
                Ok(())
            })
            .expect("expire the lease");
        let recovery = session.recover_tasks().expect("recover tasks");
        assert_eq!(recovery.requeued, 1);
        let task = session
            .task(&task_id)
            .expect("read task")
            .expect("task exists");
        assert_eq!(task.status, TaskStatus::Pending);
    }

    let retry = run(
        home.path(),
        &[
            "agent",
            home_text,
            conversation.as_str(),
            "req-agent-mid-stream",
        ],
        None,
    );
    assert!(
        retry.status.success(),
        "the retry failed: {}",
        output_text(&retry)
    );

    let harness = Harness::open(home.path()).expect("reopen harness");
    let session = harness.session();
    let history = session.history(&conversation).expect("history");
    assert_eq!(history.len(), 2, "history was {history:?}");
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[1].kind, "assistant");
    let submission = session
        .submission_by_request_id("req-agent-mid-stream")
        .expect("submission lookup")
        .expect("submission exists");
    assert_eq!(submission.status, SubmissionStatus::Done);
    assert!(submission.answer_entry_id.is_some());
}

#[test]
fn a_crash_during_consolidation_leaves_no_row_half_closed() {
    let home = home();
    let home_text = home.path().to_str().expect("utf-8 home");
    {
        let harness = Harness::open(home.path()).expect("open harness");
        let session = harness.session().clone();
        let now = session.now_micros();
        let content = "the user prefers dark mode";
        let digest = hearth_memory::fingerprint(content);
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
    }

    let crashed = run(
        home.path(),
        &["consolidate", home_text],
        Some("memory.consolidate"),
    );
    assert!(
        !crashed.status.success(),
        "the driver did not crash: {}",
        output_text(&crashed)
    );

    {
        let harness = Harness::open(home.path()).expect("reopen harness");
        let service = hearth_memory::MemoryService::new(harness.session().clone());
        assert_eq!(
            service.count().expect("count"),
            2,
            "a rolled-back pass must leave both rows current"
        );
        assert!(
            harness
                .session()
                .meta_value("last_consolidation")
                .expect("stamp lookup")
                .is_none(),
            "a rolled-back pass must not stamp the store"
        );
    }

    let retry = run(home.path(), &["consolidate", home_text], None);
    assert!(
        retry.status.success(),
        "the retry failed: {}",
        output_text(&retry)
    );
    let harness = Harness::open(home.path()).expect("reopen harness");
    let service = hearth_memory::MemoryService::new(harness.session().clone());
    assert_eq!(service.count().expect("count"), 1);
}
