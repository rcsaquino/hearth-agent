use std::path::Path;

use hearth_runtime::{InitOptions, doctor_with, init};
use tempfile::TempDir;

fn init_home(home: &Path) {
    init(
        home,
        InitOptions {
            telegram_token: "123:fake-token".to_owned(),
            allowed_user_ids: vec![42],
            base_url: "https://example.com/v1".to_owned(),
            api_key: "fake-key".to_owned(),
            model: "fake-model".to_owned(),
            thinking: "default".to_owned(),
            agent_name: "Hearth".to_owned(),
            timezone: Some("UTC".to_owned()),
            media: None,
            context_window: None,
            daemon: Default::default(),
        },
    )
    .expect("init");
}

fn run(home: &Path, live: bool) -> Result<String, String> {
    let mut output = Vec::new();
    match doctor_with(home, live, &mut output) {
        Ok(()) => Ok(String::from_utf8_lossy(&output).into_owned()),
        Err(error) => Err(error.to_string()),
    }
}

#[test]
fn doctor_reports_missing_settings_in_one_sentence() {
    let home = TempDir::new().expect("temp home");
    let error = run(home.path(), false).expect_err("missing settings");
    assert_eq!(error.lines().count(), 1, "expected one sentence: {error}");
    assert!(error.contains("settings"), "{error}");
}

#[test]
fn doctor_reports_a_missing_database() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());
    std::fs::remove_file(home.path().join("hearth.db")).expect("remove database");
    let error = run(home.path(), false).expect_err("missing database");
    assert!(error.contains("database"), "{error}");
}

#[test]
fn doctor_rejects_a_bad_settings_mode() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            home.path().join("settings.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("loosen mode");
    }
    let error = run(home.path(), false).expect_err("bad mode");
    assert!(error.contains("600"), "{error}");
}

#[test]
fn doctor_offline_never_prints_configured_secrets() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());
    let output = run(home.path(), false).expect("doctor");
    assert!(!output.contains("fake-key"), "{output}");
    assert!(!output.contains("fake-token"), "{output}");
    assert!(output.contains("lock: not held"), "{output}");
}

#[test]
fn doctor_reports_a_locked_home() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.path().join("hearth.lock"))
        .expect("open lock");
    lock.try_lock().expect("take lock");
    let output = run(home.path(), false).expect("doctor");
    assert!(output.contains("lock: held"), "{output}");
}

fn save_prompt(home: &Path, tokens: serde_json::Value) {
    let harness = hearth_core::Harness::open(home).expect("open harness");
    let document = serde_json::json!({"tokens": tokens});
    harness
        .session()
        .save_prompt_blob(&document.to_string())
        .expect("save prompt");
}

#[test]
fn doctor_names_a_prompt_segment_over_budget() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());
    save_prompt(
        home.path(),
        serde_json::json!({
            "identity": 3_200, "tools": 10, "rules": 10, "skills": 0, "guidance": 0,
            "transcript": 10, "recall": 0, "runtime": 10, "total": 3_240, "fixed": 3_230
        }),
    );
    let output = run(home.path(), false).expect("doctor");
    assert!(output.contains("identity over budget"), "{output}");
    assert!(output.contains("3200 tok of 800"), "{output}");
}

#[test]
fn doctor_reports_a_prompt_that_fits() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());
    save_prompt(
        home.path(),
        serde_json::json!({
            "identity": 800, "tools": 1_200, "rules": 800, "skills": 300, "guidance": 400,
            "transcript": 100, "recall": 300, "runtime": 150, "total": 4_050, "fixed": 3_650
        }),
    );
    let output = run(home.path(), false).expect("doctor");
    assert!(output.contains("prompt: ok"), "{output}");
    assert!(!output.contains("over budget"), "{output}");
}
