use std::path::Path;

use hearth_cli::daemon_client::{self, Ownership};
use hearth_config::Daemon;
use hearth_runtime::{InitOptions, init};
use tempfile::TempDir;

fn wait_for_stop(home: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while daemon_client::probe(home).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon did not stop in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn init_daemon_home(home: &Path) {
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
            daemon: Daemon::default(),
        },
    )
    .expect("init");
    let path = home.join("settings.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("settings")).expect("json");
    value
        .as_object_mut()
        .expect("settings object")
        .remove("telegram");
    std::fs::write(&path, serde_json::to_string_pretty(&value).expect("json")).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
}

/// Delete everything in the home except the front door.
fn strip_to_settings(home: &Path) {
    for entry in std::fs::read_dir(home).expect("home entries") {
        let path = entry.expect("entry").path();
        if path.file_name().and_then(|name| name.to_str()) == Some("settings.json") {
            continue;
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path).expect("remove directory");
        } else {
            std::fs::remove_file(&path).expect("remove file");
        }
    }
}

#[test]
fn a_home_with_only_settings_is_restored_by_the_daemon() {
    let home = TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    strip_to_settings(home.path());
    assert!(!home.path().join("hearth.db").exists());
    assert!(!home.path().join("workspace").exists());

    let executable = Path::new(env!("CARGO_BIN_EXE_hearth"));
    let (ownership, status) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("start daemon");
    assert_eq!(status["model"], "fake-model");

    assert!(home.path().join("hearth.db").is_file());
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
    let soul = std::fs::read_to_string(home.path().join("SOUL.md")).expect("SOUL.md");
    assert!(soul.contains("<identity>"), "soul was: {soul}");
    assert!(
        std::fs::read_to_string(home.path().join("HEARTH.md"))
            .expect("HEARTH.md")
            .is_empty()
    );

    let mut out = Vec::new();
    hearth_runtime::doctor_with(home.path(), false, &mut out).expect("doctor after restore");
    let text = String::from_utf8(out).expect("utf8");
    assert!(text.contains("database: ok (schema"), "{text}");

    daemon_client::stop(home.path(), &ownership).expect("stop daemon");
}

#[test]
fn ensure_starts_attaches_and_stops_the_daemon() {
    let home = TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let executable = Path::new(env!("CARGO_BIN_EXE_hearth"));

    let (ownership, status) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("start daemon");
    assert!(
        matches!(ownership, Ownership::Spawned(_)),
        "ownership was {ownership:?}"
    );
    assert_eq!(status["model"], "fake-model");

    let (second, _) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("second ensure attaches");
    assert_eq!(second, Ownership::Attached);

    daemon_client::stop_on_exit(home.path(), &ownership, true);
    assert!(
        daemon_client::probe(home.path()).is_ok(),
        "keep_running must leave the daemon alive"
    );

    daemon_client::stop(home.path(), &ownership).expect("stop daemon");
    assert!(daemon_client::probe(home.path()).is_err());
}

#[test]
fn keep_running_false_stops_a_spawned_daemon() {
    let home = TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let executable = Path::new(env!("CARGO_BIN_EXE_hearth"));
    let (ownership, _) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("start daemon");

    daemon_client::stop_on_exit(home.path(), &ownership, false);
    wait_for_stop(home.path());
    assert!(
        daemon_client::probe(home.path()).is_err(),
        "the spawned daemon should be stopped"
    );
}

#[test]
fn keep_running_false_allows_the_next_open_to_start_again() {
    let home = TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let executable = Path::new(env!("CARGO_BIN_EXE_hearth"));
    let (ownership, _) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("start daemon");

    daemon_client::stop_on_exit(home.path(), &ownership, false);
    wait_for_stop(home.path());
    assert!(
        daemon_client::probe(home.path()).is_err(),
        "the first daemon should be stopped"
    );

    let (second, status) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("start again");
    assert!(
        matches!(second, Ownership::Spawned(_)),
        "ownership was {second:?}"
    );
    assert_eq!(status["model"], "fake-model");
    daemon_client::stop(home.path(), &second).expect("stop daemon");
}

#[test]
fn an_attached_daemon_can_be_stopped_from_the_tui() {
    let home = TempDir::new().expect("temp home");
    init_daemon_home(home.path());
    let executable = Path::new(env!("CARGO_BIN_EXE_hearth"));
    let (spawned, _) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("start daemon");
    let (attached, _) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("attach to daemon");
    assert_eq!(attached, Ownership::Attached);

    daemon_client::stop(home.path(), &attached).expect("stop an attached daemon");
    wait_for_stop(home.path());

    daemon_client::stop_on_exit(home.path(), &spawned, true);
}
