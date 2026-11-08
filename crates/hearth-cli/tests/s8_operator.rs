use std::process::Command;

fn hearth() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hearth"))
}

#[test]
fn help_lists_the_m0_commands() {
    let output = hearth().arg("--help").output().expect("run hearth");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("init"), "help did not mention init: {text}");
    assert!(
        text.contains("daemon"),
        "help did not mention daemon: {text}"
    );
    assert!(
        text.contains("doctor"),
        "help did not mention doctor: {text}"
    );
}

#[test]
fn doctor_without_settings_fails_with_one_sentence() {
    let home = tempfile::TempDir::new().expect("temp home");
    let output = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.trim();
    assert_eq!(line.lines().count(), 1, "expected one sentence: {text}");
    assert!(
        line.to_lowercase().contains("settings"),
        "unexpected message: {line}"
    );
}

fn init_home(home: &std::path::Path) {
    let output = hearth()
        .args(["init", "--non-interactive"])
        .arg("--home")
        .arg(home)
        .args(["--telegram-token", "123:fake-token"])
        .args(["--allowed-user-id", "42"])
        .args(["--base-url", "https://example.com/v1"])
        .args(["--api-key", "fake-key"])
        .args(["--model", "fake-model"])
        .output()
        .expect("run init");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_settings(home: &std::path::Path, text: &str) {
    let path = home.join("settings.json");
    std::fs::write(&path, text).expect("write settings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("set settings mode");
    }
}

#[test]
fn doctor_rejects_invalid_json() {
    let home = tempfile::TempDir::new().expect("temp home");
    write_settings(home.path(), "{ not json");
    let output = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.trim();
    assert_eq!(line.lines().count(), 1, "expected one sentence: {text}");
    assert!(
        line.to_lowercase().contains("json"),
        "unexpected message: {line}"
    );
}

#[test]
fn doctor_rejects_an_unknown_setting() {
    let home = tempfile::TempDir::new().expect("temp home");
    write_settings(
        home.path(),
        r#"{
            "identity": {"agent_name": "Hearth", "user_name": "User", "timezone": "UTC"},
            "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake",
                "model": "fake-model"
            },
            "nope": true
        }"#,
    );
    let output = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.trim();
    assert_eq!(line.lines().count(), 1, "expected one sentence: {text}");
    assert!(
        line.to_lowercase().contains("unknown"),
        "unexpected message: {line}"
    );
}

#[test]
fn doctor_accepts_a_complete_settings_file() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path());
    write_settings(
        home.path(),
        r#"{
            "identity": {
                "agent_name": "Hearth",
                "user_name": "User",
                "timezone": "UTC"
            },
            "telegram": {
                "bot_token": "123:fake",
                "allowed_user_ids": [42],
                "allowed_chat_ids": [-100]
            },
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake",
                "model": "fake-model",
                "session_header": "x-opencode-session",
                "headers": {"x-extra": "value"}
            },
            "media": {
                "image": {
                    "kind": "openrouter-images",
                    "api_key": "fake",
                    "model": "fake-image"
                },
                "stt": {
                    "kind": "groq",
                    "api_key": "fake",
                    "model": "fake-stt"
                },
                "tts": {
                    "kind": "elevenlabs",
                    "api_key": "fake",
                    "model": "eleven_v4",
                    "voice_id": "fake-voice",
                    "output_format": "opus_48000_64"
                }
            },
            "tasks": {
                "max_concurrent": 4,
                "max_depth": 2,
                "retention_days": 30,
                "retention_runs": 25,
                "eta_reminder_minutes": 10
            },
            "web": {
                "search": {
                    "order": ["exa", "brave"],
                    "parallel": 3,
                    "results": 5,
                    "max_results": 10,
                    "providers": {"brave": {"api_key": "fake"}}
                }
            },
            "agent": {"workspace": "/tmp/hearth-workspace", "max_turns": 30},
            "compaction": {"idle_minutes": 60, "idle_min_tokens": 100000},
            "memory": {"standing_max_tokens": 1200, "recall_max_tokens": 900},
            "logging": {"level": "info"}
        }"#,
    );
    let output = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn doctor_names_a_missing_environment_variable() {
    let home = tempfile::TempDir::new().expect("temp home");
    write_settings(
        home.path(),
        r#"{
            "identity": {"agent_name": "Hearth", "user_name": "User", "timezone": "UTC"},
            "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
            "models": {
                "kind": "openai-compatible",
                "base_url": "${HEARTH_TEST_MISSING}",
                "api_key": "fake",
                "model": "fake-model"
            }
        }"#,
    );
    let output = hearth()
        .env_remove("HEARTH_TEST_MISSING")
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.trim();
    assert_eq!(line.lines().count(), 1, "expected one sentence: {text}");
    assert!(
        line.contains("HEARTH_TEST_MISSING"),
        "unexpected message: {line}"
    );
}

#[test]
fn doctor_resolves_environment_references() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path());
    write_settings(
        home.path(),
        r#"{
            "identity": {"agent_name": "Hearth", "user_name": "User", "timezone": "UTC"},
            "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
            "models": {
                "kind": "openai-compatible",
                "base_url": "${HEARTH_TEST_URL}",
                "api_key": "fake",
                "model": "fake-model"
            }
        }"#,
    );
    let output = hearth()
        .env("HEARTH_TEST_URL", "https://example.com/v1")
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn doctor_refuses_settings_that_are_not_0600() {
    let home = tempfile::TempDir::new().expect("temp home");
    write_settings(
        home.path(),
        r#"{
            "identity": {"agent_name": "Hearth", "user_name": "User", "timezone": "UTC"},
            "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake",
                "model": "fake-model"
            }
        }"#,
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            home.path().join("settings.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("loosen mode");
    }
    let output = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.trim();
    assert_eq!(line.lines().count(), 1, "expected one sentence: {text}");
    assert!(line.contains("mode"), "unexpected message: {line}");
    assert!(line.contains("600"), "unexpected message: {line}");
}

#[test]
fn init_creates_a_working_home() {
    let home = tempfile::TempDir::new().expect("temp home");
    let output = hearth()
        .args(["init", "--non-interactive"])
        .arg("--home")
        .arg(home.path())
        .args(["--telegram-token", "123:fake-token"])
        .args(["--allowed-user-id", "42"])
        .args(["--base-url", "https://opencode.ai/inference/go/openai/v1"])
        .args(["--api-key", "fake-key"])
        .args(["--model", "fake-model"])
        .output()
        .expect("run hearth");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("fake-token"), "init echoed the token");
    assert!(!stdout.contains("fake-key"), "init echoed the api key");

    let settings_path = home.path().join("settings.json");
    let settings = std::fs::read_to_string(&settings_path).expect("read settings");
    let json: serde_json::Value = serde_json::from_str(&settings).expect("settings JSON");
    assert_eq!(json["identity"]["agent_name"], "Hearth");
    assert_eq!(json["identity"]["user_name"], "User");
    assert_eq!(json["telegram"]["allowed_user_ids"][0], 42);
    assert_eq!(json["models"]["session_header"], "x-opencode-session");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&settings_path)
            .expect("settings metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "settings mode was {mode:o}");
    }

    for directory in [
        "workspace",
        "skills",
        "skills/learned",
        "media/inbound",
        "media/outbound",
        "spill",
        "logs",
        "backups",
    ] {
        assert!(
            home.path().join(directory).is_dir(),
            "missing directory {directory}"
        );
    }
    let database = home.path().join("hearth.db");
    assert!(database.is_file(), "missing database");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&database)
            .expect("database metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "database mode was {mode:o}");
    }

    let doctor = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run doctor");
    assert!(
        doctor.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&doctor.stderr)
    );
}

#[test]
fn init_refuses_to_overwrite_an_existing_settings_file() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path());
    let output = hearth()
        .args(["init", "--non-interactive"])
        .arg("--home")
        .arg(home.path())
        .args(["--telegram-token", "123:other-token"])
        .args(["--allowed-user-id", "43"])
        .args(["--base-url", "https://example.com/v1"])
        .args(["--api-key", "other-key"])
        .args(["--model", "other-model"])
        .output()
        .expect("run init");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(
        text.to_lowercase().contains("exists"),
        "unexpected message: {text}"
    );
}
