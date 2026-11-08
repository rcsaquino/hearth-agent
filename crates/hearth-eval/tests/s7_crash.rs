use std::path::Path;
use std::process::{Command, Output};

use hearth_core::{EffectIntent, EffectStatus, Harness, Save};
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
fn a_crash_during_an_effect_replays_it_once() {
    let home = home();
    let home_text = home.path().to_str().expect("utf-8 home");
    let conversation = conversation(home.path());
    let effect_id = {
        let harness = Harness::open(home.path()).expect("open harness");
        let commit = harness
            .session()
            .save(Save::new(&conversation).effect(EffectIntent::new(
                "file_write",
                "s7-effect",
                serde_json::json!({"path": "s7.txt", "content": "hello"}),
            )))
            .expect("save effect intent");
        commit.effect_ids[0].clone()
    };
    let workspace = home.path().join("workspace");
    let workspace_text = workspace.to_str().expect("utf-8 workspace");

    let crashed = run(
        home.path(),
        &["effects", home_text, workspace_text],
        Some("during_effect"),
    );
    assert!(
        !crashed.status.success(),
        "the driver did not crash: {}",
        output_text(&crashed)
    );

    {
        let harness = Harness::open(home.path()).expect("reopen harness");
        let effect = harness
            .session()
            .effect(&effect_id)
            .expect("read effect")
            .expect("effect exists");
        assert_eq!(effect.status, EffectStatus::Started);
        assert_eq!(effect.attempts, 1);
        assert_eq!(
            std::fs::read_to_string(workspace.join("s7.txt")).expect("read file"),
            "hello"
        );
        let commit = harness
            .session()
            .latest_commit()
            .expect("latest commit")
            .expect("a commit exists");
        assert_eq!(commit.kind, "effect.started");
    }

    let retry = run(home.path(), &["effects", home_text, workspace_text], None);
    assert!(
        retry.status.success(),
        "the effect retry failed: {}",
        output_text(&retry)
    );

    let harness = Harness::open(home.path()).expect("reopen harness");
    let effect = harness
        .session()
        .effect(&effect_id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Done);
    assert_eq!(effect.attempts, 2);
    assert_eq!(
        std::fs::read_to_string(workspace.join("s7.txt")).expect("read file"),
        "hello"
    );
    let commit = harness
        .session()
        .latest_commit()
        .expect("latest commit")
        .expect("a commit exists");
    assert_eq!(commit.kind, "effect.done");
}
