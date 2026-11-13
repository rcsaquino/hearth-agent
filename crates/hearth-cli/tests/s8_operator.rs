use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use hearth_config::Daemon;
use hearth_runtime::{InitOptions, init};

fn hearth() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hearth"))
}

fn hearth_daemon(home: &Path) -> Command {
    let mut command = hearth();
    command
        .env("HEARTH_DAEMON", "1")
        .env("HEARTH_HOME", home)
        .stdin(Stdio::null());
    command
}

fn init_home(home: &std::path::Path) {
    init_home_with_base_url(home, "https://example.com/v1");
}

fn init_home_with_base_url(home: &std::path::Path, base_url: &str) {
    init(
        home,
        InitOptions {
            telegram_token: "123:fake-token".to_owned(),
            allowed_user_ids: vec![42],
            base_url: base_url.to_owned(),
            api_key: "fake-key".to_owned(),
            model: "fake-model".to_owned(),
            thinking: "default".to_owned(),
            agent_name: "Hearth".to_owned(),
            timezone: Some("UTC".to_owned()),
            media: None,
            context_window: None,
            daemon: Daemon::default(),
        },
    )
    .expect("init home");
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
    write_settings(
        home,
        &serde_json::to_string_pretty(&value).expect("settings json"),
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

fn wait_for_lock_pid(home: &Path, child: &mut Child) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(20);
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
    let deadline = Instant::now() + Duration::from_secs(20);
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
    let mut child = hearth_daemon(home.path())
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
    let mut first = hearth_daemon(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first daemon");
    let pid = wait_for_lock_pid(home.path(), &mut first);

    let second = hearth_daemon(home.path())
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
    let mut first = hearth_daemon(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first daemon");
    let first_pid = wait_for_lock_pid(home.path(), &mut first);
    terminate(first);

    let mut second = hearth_daemon(home.path())
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
    let mut first = hearth_daemon(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first daemon");
    let first_pid = wait_for_lock_pid(home.path(), &mut first);
    first.kill().expect("kill first daemon");
    first.wait().expect("reap first daemon");

    let mut second = hearth_daemon(home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn second daemon");
    let second_pid = wait_for_lock_pid(home.path(), &mut second);
    assert_ne!(first_pid, second_pid);
    terminate(second);
}

fn base_settings() -> serde_json::Value {
    serde_json::json!({
        "identity": {"agent_name": "Hearth", "timezone": "UTC"},
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
    let error = hearth_config::load(home.path()).expect_err("settings should fail");
    assert!(
        error.to_string().contains(needle),
        "unexpected message: {error}"
    );
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
    let settings = hearth_config::load(home.path()).expect("empty settings parse");
    let error = settings
        .validate_for_run()
        .expect_err("empty settings cannot run");
    assert!(error.to_string().contains("models"), "{error}");
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
    let mut daemon = hearth_daemon(home.path())
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
        hearth_daemon(home.path())
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
        hearth_daemon(home.path())
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
        hearth_daemon(home.path())
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
        hearth_daemon(home.path())
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
        hearth_daemon(home.path())
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
        hearth_daemon(home.path())
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
        hearth_daemon(home.path())
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
