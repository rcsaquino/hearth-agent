use std::fs;

use hearth_config::{ConfigError, Daemon, Settings, load, save_daemon};
use tempfile::TempDir;

fn minimal() -> &'static str {
    r#"{
        "identity": {"agent_name": "Hearth", "timezone": "UTC"},
        "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
        "models": {
            "kind": "openai-compatible",
            "base_url": "https://example.com/v1",
            "api_key": "fake-key",
            "model": "fake-model"
        }
    }"#
}

fn write(home: &TempDir, text: &str) {
    let path = home.path().join("settings.json");
    fs::write(&path, text).expect("write settings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode");
    }
}

#[test]
fn daemon_defaults_to_tui_and_keep_running() {
    let settings: Settings = serde_json::from_str(minimal()).expect("settings");
    settings.validate().expect("valid");
    assert_eq!(settings.daemon.start, "tui");
    assert!(settings.daemon.keep_running);
}

#[test]
fn daemon_accepts_each_start_mode() {
    for start in ["tui", "login", "startup"] {
        let text = minimal().replacen(
            "\"models\"",
            &format!("\"daemon\": {{\"start\": \"{start}\", \"keep_running\": false}}, \"models\""),
            1,
        );
        let settings: Settings = serde_json::from_str(&text).expect("settings");
        settings.validate().expect("valid");
        assert_eq!(settings.daemon.start, start);
        assert!(!settings.daemon.keep_running);
    }
}

#[test]
fn daemon_rejects_an_unknown_start_mode() {
    let text = minimal().replacen(
        "\"models\"",
        "\"daemon\": {\"start\": \"sometimes\"}, \"models\"",
        1,
    );
    let settings: Settings = serde_json::from_str(&text).expect("settings");
    let error = settings.validate().expect_err("invalid start");
    assert!(error.to_string().contains("daemon.start"), "{error}");
}

#[test]
fn save_daemon_preserves_the_rest_and_updates_the_section() {
    let home = TempDir::new().expect("temp home");
    write(&home, minimal());

    save_daemon(
        home.path(),
        &Daemon {
            start: "login".to_owned(),
            keep_running: false,
        },
    )
    .expect("save daemon");

    let settings = load(home.path()).expect("reload");
    assert_eq!(settings.daemon.start, "login");
    assert!(!settings.daemon.keep_running);
    assert_eq!(
        settings.models.as_ref().expect("models").model,
        "fake-model"
    );
    assert_eq!(
        settings
            .telegram
            .as_ref()
            .expect("telegram")
            .allowed_user_ids,
        vec![42]
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(home.path().join("settings.json"))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "settings mode was {mode:o}");
    }
}

#[test]
fn save_daemon_requires_an_existing_settings_file() {
    let home = TempDir::new().expect("temp home");
    let error = save_daemon(home.path(), &Daemon::default()).expect_err("missing settings");
    assert!(matches!(error, ConfigError::Missing(_)), "{error}");
}
