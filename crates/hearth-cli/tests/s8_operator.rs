use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    init_home_with_base_url(home, "https://example.com/v1");
}

fn init_home_with_base_url(home: &std::path::Path, base_url: &str) {
    let output = hearth()
        .args(["init", "--non-interactive"])
        .arg("--home")
        .arg(home)
        .args(["--telegram-token", "123:fake-token"])
        .args(["--allowed-user-id", "42"])
        .args(["--base-url", base_url])
        .args(["--api-key", "fake-key"])
        .args(["--model", "fake-model"])
        .args(["--timezone", "UTC"])
        .output()
        .expect("run init");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_daemon_home(home: &std::path::Path) {
    init_home(home);
    let path = home.join("settings.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read settings"))
            .expect("parse settings");
    if let Some(object) = value.as_object_mut() {
        object.remove("telegram");
    }
    write_settings(home, &serde_json::to_string_pretty(&value).expect("settings json"));
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
        .args(["--timezone", "UTC"])
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
fn init_uses_the_timezone_from_the_environment() {
    let home = tempfile::TempDir::new().expect("temp home");
    let output = hearth()
        .args(["init", "--non-interactive"])
        .arg("--home")
        .arg(home.path())
        .args(["--telegram-token", "123:fake-token"])
        .args(["--allowed-user-id", "42"])
        .args(["--base-url", "https://example.com/v1"])
        .args(["--api-key", "fake-key"])
        .args(["--model", "fake-model"])
        .env("TZ", "Asia/Manila")
        .output()
        .expect("run hearth");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let settings =
        std::fs::read_to_string(home.path().join("settings.json")).expect("read settings");
    let json: serde_json::Value = serde_json::from_str(&settings).expect("settings JSON");
    assert_eq!(json["identity"]["timezone"], "Asia/Manila");
}

#[test]
fn init_creates_the_default_soul() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path());

    let soul_path = home.path().join("SOUL.md");
    let soul = std::fs::read_to_string(&soul_path).expect("read SOUL.md");
    assert!(soul.contains("# Identity"), "missing Identity section");
    assert!(soul.contains("# Voice"), "missing Voice section");
    assert!(soul.contains("# Rules"), "missing Rules section");
    assert!(soul.contains("Hearth"), "missing agent name");
    assert!(soul.contains("User"), "missing user name");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&soul_path)
            .expect("SOUL metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "SOUL mode was {mode:o}");
    }
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

fn remove_database(home: &std::path::Path) {
    for name in ["hearth.db", "hearth.db-wal", "hearth.db-shm"] {
        let path = home.join(name);
        if path.exists() {
            std::fs::remove_file(path).expect("remove database file");
        }
    }
}

#[test]
fn doctor_reports_a_missing_database() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path());
    remove_database(home.path());
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
        line.to_lowercase().contains("database"),
        "unexpected message: {line}"
    );
}

#[test]
fn doctor_reports_a_corrupt_database() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path());
    std::fs::write(home.path().join("hearth.db"), "this is not a database")
        .expect("corrupt database");
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
        line.to_lowercase().contains("database"),
        "unexpected message: {line}"
    );
}

#[test]
fn doctor_does_not_print_secrets() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path());
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("settings: ok"), "missing settings line");
    assert!(stdout.contains("database: ok"), "missing database line");
    for secret in ["fake-token", "fake-key"] {
        assert!(!stdout.contains(secret), "stdout leaked {secret}");
        assert!(!stderr.contains(secret), "stderr leaked {secret}");
    }
}

fn wait_for_lock_pid(home: &Path, child: &mut Child) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("poll daemon") {
            panic!("daemon exited before writing the lock: {status}");
        }
        if let Ok(text) = std::fs::read_to_string(home.join("hearth.lock"))
            && let Ok(pid) = text.trim().parse::<u32>()
            && pid == child.id()
        {
            return pid;
        }
        if Instant::now() >= deadline {
            panic!("daemon did not write the lock file");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn terminate(mut child: Child) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait().expect("poll daemon") {
            Some(_) => return,
            None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            None => {
                let _ = child.kill();
                panic!("daemon did not stop on SIGTERM");
            }
        }
    }
}

#[test]
fn daemon_writes_its_pid_to_the_lock_file() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let mut child = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let pid = wait_for_lock_pid(home.path(), &mut child);
    assert_eq!(pid, child.id());
    terminate(child);
}

#[test]
fn second_daemon_refuses_to_start() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let mut first = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first daemon");
    let pid = wait_for_lock_pid(home.path(), &mut first);

    let second = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run second daemon");
    assert!(!second.status.success());
    let text = String::from_utf8_lossy(&second.stderr);
    let line = text.trim();
    assert_eq!(line.lines().count(), 1, "expected one sentence: {text}");
    assert!(
        line.contains("already running"),
        "unexpected message: {line}"
    );
    assert!(line.contains(&pid.to_string()), "missing pid: {line}");

    terminate(first);
}

#[test]
fn daemon_releases_the_lock_on_shutdown() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let mut first = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first daemon");
    let first_pid = wait_for_lock_pid(home.path(), &mut first);
    terminate(first);

    let mut second = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn second daemon");
    let second_pid = wait_for_lock_pid(home.path(), &mut second);
    assert_ne!(first_pid, second_pid);
    terminate(second);
}

#[test]
fn a_stale_lock_does_not_block_a_new_daemon() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let mut first = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first daemon");
    let first_pid = wait_for_lock_pid(home.path(), &mut first);
    first.kill().expect("kill first daemon");
    first.wait().expect("reap first daemon");

    let mut second = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn second daemon");
    let second_pid = wait_for_lock_pid(home.path(), &mut second);
    assert_ne!(first_pid, second_pid);
    terminate(second);
}

#[test]
fn doctor_reports_the_lock_state() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());

    let idle = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run doctor");
    assert!(idle.status.success());
    let idle_stdout = String::from_utf8_lossy(&idle.stdout);
    assert!(
        idle_stdout.contains("lock: not held"),
        "unexpected output: {idle_stdout}"
    );

    let mut daemon = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let pid = wait_for_lock_pid(home.path(), &mut daemon);

    let held = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run doctor");
    assert!(held.status.success());
    let held_stdout = String::from_utf8_lossy(&held.stdout);
    assert!(
        held_stdout.contains("lock: held"),
        "unexpected output: {held_stdout}"
    );
    assert!(
        held_stdout.contains(&pid.to_string()),
        "missing pid: {held_stdout}"
    );

    terminate(daemon);

    let released = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run doctor");
    assert!(released.status.success());
    let released_stdout = String::from_utf8_lossy(&released.stdout);
    assert!(
        released_stdout.contains("lock: not held"),
        "unexpected output: {released_stdout}"
    );
}

fn base_settings() -> serde_json::Value {
    serde_json::json!({
        "identity": {"agent_name": "Hearth", "user_name": "User", "timezone": "UTC"},
        "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
        "models": {
            "kind": "openai-compatible",
            "base_url": "https://example.com/v1",
            "api_key": "fake",
            "model": "fake-model"
        }
    })
}

fn write_value(home: &Path, value: &serde_json::Value) {
    write_settings(
        home,
        &serde_json::to_string_pretty(value).expect("settings JSON"),
    );
}

fn expect_settings_failure(value: &serde_json::Value, needle: &str) {
    let home = tempfile::TempDir::new().expect("temp home");
    write_value(home.path(), value);
    let output = hearth()
        .arg("doctor")
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run hearth");
    assert!(!output.status.success(), "doctor unexpectedly passed");
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.trim();
    assert_eq!(line.lines().count(), 1, "expected one sentence: {text}");
    assert!(line.contains(needle), "unexpected message: {line}");
}

#[test]
fn telegram_requires_an_allowed_user() {
    let mut value = base_settings();
    value["telegram"]["allowed_user_ids"] = serde_json::json!([]);
    expect_settings_failure(&value, "telegram.allowed_user_ids");
}

#[test]
fn an_empty_settings_file_parses_but_cannot_run() {
    let home = tempfile::TempDir::new().expect("temp home");
    write_settings(home.path(), "");
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
    assert!(line.contains("models"), "unexpected message: {line}");
}

#[test]
fn media_image_requires_an_api_key() {
    let mut value = base_settings();
    value["media"] = serde_json::json!({
        "image": {"kind": "openrouter-images", "model": "fake-image"}
    });
    expect_settings_failure(&value, "media.image.api_key");
}

#[test]
fn media_tts_requires_a_voice_id() {
    let mut value = base_settings();
    value["media"] = serde_json::json!({
        "tts": {"kind": "elevenlabs", "api_key": "fake", "model": "eleven_v4"}
    });
    expect_settings_failure(&value, "media.tts.voice_id");
}

#[test]
fn media_base_url_must_use_https() {
    let mut value = base_settings();
    value["media"] = serde_json::json!({
        "image": {
            "kind": "openrouter-images",
            "api_key": "fake",
            "model": "fake-image",
            "base_url": "http://example.com"
        }
    });
    expect_settings_failure(&value, "must use https");
}

#[test]
fn media_stt_requires_an_api_key() {
    let mut value = base_settings();
    value["media"] = serde_json::json!({
        "stt": {"kind": "groq", "model": "fake-stt"}
    });
    expect_settings_failure(&value, "media.stt.api_key");
}

#[test]
fn web_search_rejects_an_unknown_provider() {
    let mut value = base_settings();
    value["web"] = serde_json::json!({"search": {"order": ["nope"]}});
    expect_settings_failure(&value, "unknown provider");
}

#[test]
fn web_search_rejects_a_bad_range() {
    let mut value = base_settings();
    value["web"] = serde_json::json!({"search": {"order": ["exa"], "parallel": 0}});
    expect_settings_failure(&value, "web.search.parallel");
}

#[test]
fn tasks_rejects_a_bad_range() {
    let mut value = base_settings();
    value["tasks"] = serde_json::json!({"max_concurrent": 9});
    expect_settings_failure(&value, "tasks.max_concurrent");
}

#[test]
fn models_rejects_an_unknown_kind() {
    let mut value = base_settings();
    value["models"]["kind"] = serde_json::json!("other");
    expect_settings_failure(&value, "models.kind");
}

#[test]
fn logging_rejects_an_unknown_level() {
    let mut value = base_settings();
    value["logging"] = serde_json::json!({"level": "loud"});
    expect_settings_failure(&value, "logging.level");
}

fn wait_for_log(home: &Path, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(home.join("logs/hearth.log"))
            && text.contains(needle)
        {
            return;
        }
        if Instant::now() >= deadline {
            panic!("log did not contain {needle:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn daemon_writes_a_structured_log() {
    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let mut daemon = hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    wait_for_lock_pid(home.path(), &mut daemon);
    wait_for_log(home.path(), "hearth daemon started");
    terminate(daemon);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.path().join("logs/hearth.log"))
            .expect("log metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "log mode was {mode:o}");
    }

    let log = std::fs::read_to_string(home.path().join("logs/hearth.log")).expect("read log");
    assert!(log.contains("\"pid\""), "log has no pid field: {log}");
    assert!(
        log.lines().all(|line| line.trim_start().starts_with('{')),
        "log is not JSON lines: {log}"
    );
}

struct DaemonGuard(Option<Child>);

impl DaemonGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child(&mut self) -> &mut Child {
        self.0.as_mut().expect("daemon child")
    }

    fn stop(mut self) {
        if let Some(child) = self.0.take() {
            terminate(child);
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn daemon_recovers_an_expired_task_lease() {
    use hearth_core::{Harness, NewTask, TaskStatus};

    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());

    let task_id = {
        let harness = Harness::open(home.path()).expect("open harness");
        let session = harness.session();
        let task = session
            .create_task(NewTask::new(
                "test.recovery",
                1,
                "start",
                serde_json::json!({"step": 1}),
            ))
            .expect("create task");
        let claimed = session
            .claim_task("test-owner", 1)
            .expect("claim task")
            .expect("task is claimable");
        assert_eq!(claimed.id, task.id);
        std::thread::sleep(Duration::from_millis(5));
        task.id
    };

    let mut daemon = DaemonGuard::new(
        hearth()
            .args(["daemon"])
            .arg("--home")
            .arg(home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon"),
    );
    wait_for_lock_pid(home.path(), daemon.child());
    wait_for_log(home.path(), "task recovery complete");
    daemon.stop();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let task = harness
        .session()
        .task(&task_id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Pending);
    assert!(task.lease_owner.is_none());
    assert!(task.lease_until.is_none());
    let commit = harness
        .session()
        .latest_commit()
        .expect("latest commit")
        .expect("a commit exists");
    assert_eq!(commit.kind, "task.recover");
}

fn seed_effect(home: &Path, kind: &str, key: &str, payload: serde_json::Value) -> String {
    use hearth_core::{EffectIntent, Harness, Save};

    let harness = Harness::open(home).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let commit = session
        .save(Save::new(&conversation.id).effect(EffectIntent::new(kind, key, payload)))
        .expect("save effect intent");
    commit.effect_ids[0].clone()
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if path.exists() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("file was not written: {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn daemon_executes_a_file_write_effect() {
    use hearth_core::{EffectStatus, Harness};

    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let effect_id = seed_effect(
        home.path(),
        "file_write",
        "file-1",
        serde_json::json!({"path": "notes/hello.txt", "content": "hello"}),
    );

    let mut daemon = DaemonGuard::new(
        hearth()
            .args(["daemon"])
            .arg("--home")
            .arg(home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon"),
    );
    wait_for_lock_pid(home.path(), daemon.child());
    let target = home.path().join("workspace/notes/hello.txt");
    wait_for_file(&target);
    daemon.stop();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let effect = harness
        .session()
        .effect(&effect_id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Done);
    let result = effect.result.expect("effect result");
    assert_eq!(result["path"], "notes/hello.txt");
    assert_eq!(result["bytes"], 5);
    assert_eq!(
        std::fs::read_to_string(&target).expect("read file"),
        "hello"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target)
            .expect("file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "file mode was {mode:o}");
    }
}

#[test]
fn daemon_fails_a_path_escape_effect() {
    use hearth_core::{EffectStatus, Harness};

    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let effect_id = seed_effect(
        home.path(),
        "file_write",
        "escape-1",
        serde_json::json!({"path": "../escape.txt", "content": "bad"}),
    );

    let mut daemon = DaemonGuard::new(
        hearth()
            .args(["daemon"])
            .arg("--home")
            .arg(home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon"),
    );
    wait_for_lock_pid(home.path(), daemon.child());
    wait_for_log(home.path(), "effect failed");
    daemon.stop();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let effect = harness
        .session()
        .effect(&effect_id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Failed);
    let result = effect.result.expect("effect result");
    assert_eq!(result["retryable"], false);
    assert!(
        !home.path().join("escape.txt").exists(),
        "an escaped file was written"
    );
}

#[test]
fn daemon_fails_an_unknown_effect_kind() {
    use hearth_core::{EffectStatus, Harness};

    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let effect_id = seed_effect(
        home.path(),
        "unsupported.effect",
        "unknown-1",
        serde_json::json!({}),
    );

    let mut daemon = DaemonGuard::new(
        hearth()
            .args(["daemon"])
            .arg("--home")
            .arg(home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon"),
    );
    wait_for_lock_pid(home.path(), daemon.child());
    wait_for_log(home.path(), "effect failed");
    daemon.stop();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let effect = harness
        .session()
        .effect(&effect_id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Failed);
    let result = effect.result.expect("effect result");
    assert_eq!(result["retryable"], false);
}

#[test]
fn daemon_requeues_a_started_file_write_effect() {
    use hearth_core::{EffectStatus, Harness};

    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let effect_id = seed_effect(
        home.path(),
        "file_write",
        "started-1",
        serde_json::json!({"path": "notes/started.txt", "content": "again"}),
    );
    {
        let harness = Harness::open(home.path()).expect("open harness");
        harness
            .session()
            .begin_effect(&effect_id)
            .expect("begin effect");
    }

    let mut daemon = DaemonGuard::new(
        hearth()
            .args(["daemon"])
            .arg("--home")
            .arg(home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon"),
    );
    wait_for_lock_pid(home.path(), daemon.child());
    let target = home.path().join("workspace/notes/started.txt");
    wait_for_file(&target);
    daemon.stop();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let effect = harness
        .session()
        .effect(&effect_id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Done);
    assert_eq!(effect.attempts, 2);
    assert_eq!(
        std::fs::read_to_string(&target).expect("read file"),
        "again"
    );
}

#[cfg(unix)]
#[test]
fn daemon_fails_a_symlink_escape_effect() {
    use hearth_core::{EffectStatus, Harness};

    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let outside = tempfile::TempDir::new().expect("outside");
    std::os::unix::fs::symlink(outside.path(), home.path().join("workspace/link"))
        .expect("create symlink");
    let effect_id = seed_effect(
        home.path(),
        "file_write",
        "symlink-1",
        serde_json::json!({"path": "link/escape.txt", "content": "bad"}),
    );

    let mut daemon = DaemonGuard::new(
        hearth()
            .args(["daemon"])
            .arg("--home")
            .arg(home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon"),
    );
    wait_for_lock_pid(home.path(), daemon.child());
    wait_for_log(home.path(), "effect failed");
    daemon.stop();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let effect = harness
        .session()
        .effect(&effect_id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Failed);
    assert!(
        !outside.path().join("escape.txt").exists(),
        "a file escaped through a symlink"
    );
}

#[test]
fn daemon_stops_retrying_a_retryable_effect_after_three_attempts() {
    use hearth_core::{EffectStatus, Harness};

    let home = tempfile::TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let effect_id = seed_effect(
        home.path(),
        "file_write",
        "retry-1",
        serde_json::json!({"path": "notes/retry.txt", "content": "x"}),
    );
    std::fs::remove_dir_all(home.path().join("workspace")).expect("remove workspace");
    std::fs::write(home.path().join("workspace"), "not a directory").expect("block workspace");

    let mut daemon = DaemonGuard::new(
        hearth()
            .args(["daemon"])
            .arg("--home")
            .arg(home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon"),
    );
    wait_for_lock_pid(home.path(), daemon.child());
    wait_for_log(home.path(), "\"retryable\":false");
    daemon.stop();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let effect = harness
        .session()
        .effect(&effect_id)
        .expect("read effect")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Failed);
    assert_eq!(effect.attempts, 3);
    let result = effect.result.expect("effect result");
    assert_eq!(result["retryable"], false);
}

fn fake_sse_server(answer: &str) -> (String, thread::JoinHandle<Option<serde_json::Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake provider");
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    let port = listener.local_addr().expect("server address").port();
    let answer = answer.to_owned();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        socket
            .set_nonblocking(false)
            .expect("set accepted socket blocking");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        let mut reader = BufReader::new(socket.try_clone().expect("clone socket"));
        let mut request_line = String::new();
        reader.read_line(&mut request_line).expect("request line");
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).expect("request header") == 0 {
                break;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).expect("request body");
        let request: serde_json::Value = serde_json::from_slice(&body).expect("request JSON");

        let mut response = String::from(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        );
        response.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {"content": answer}}]})
        ));
        response.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
        ));
        response.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}})
        ));
        response.push_str("data: [DONE]\n\n");
        socket
            .write_all(response.as_bytes())
            .expect("write response");
        socket.flush().ok();
        Some(request)
    });
    (format!("http://127.0.0.1:{port}/v1"), handle)
}

#[test]
fn chat_runs_one_line_through_the_real_provider_adapter() {
    let (base_url, server) = fake_sse_server("hello from the fake");
    let home = tempfile::TempDir::new().expect("temp home");
    init_home_with_base_url(home.path(), &base_url);

    let mut child = hearth()
        .args(["chat", "--home"])
        .arg(home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn chat");
    {
        let stdin = child.stdin.as_mut().expect("chat stdin");
        stdin.write_all(b"hello\n").expect("write chat input");
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("chat output");
    let request = server
        .join()
        .expect("join fake provider")
        .expect("provider request");

    assert!(
        output.status.success(),
        "chat failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello from the fake"),
        "chat output was: {stdout}"
    );
    assert_eq!(request["model"], "fake-model");
    assert_eq!(request["stream"], true);
    assert!(
        request["messages"]
            .as_array()
            .is_some_and(|messages| !messages.is_empty()),
        "request has no messages: {request}"
    );
    assert!(
        request.to_string().contains("hello"),
        "request did not carry the user text: {request}"
    );
    let tool_names: Vec<&str> = request["tools"]
        .as_array()
        .expect("request tools")
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect();
    assert!(
        tool_names.contains(&"web_search"),
        "chat tools were: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"web_fetch"),
        "chat tools were: {tool_names:?}"
    );
}
