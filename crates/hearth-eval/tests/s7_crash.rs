use std::path::Path;
use std::process::{Command, Output};

use hearth_core::Harness;
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
