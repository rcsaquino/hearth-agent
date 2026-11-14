use std::path::Path;

use hearth_config::Daemon;
use hearth_runtime::{InitOptions, init, restore_home};
use serde_json::json;
use tempfile::TempDir;

/// The directories onboarding writes. A restored home must hold every one.
const LAYOUT: [&str; 8] = [
    "workspace",
    "skills",
    "skills/learned",
    "media/inbound",
    "media/outbound",
    "spill",
    "logs",
    "backups",
];

fn write_settings(home: &Path) {
    let value = json!({
        "identity": {"agent_name": "Hearth", "timezone": "UTC"},
        "models": {
            "kind": "openai-compatible",
            "base_url": "https://example.com/v1",
            "api_key": "fake-key",
            "model": "fake-model",
            "thinking": "default"
        },
        "daemon": {"start": "tui", "keep_running": false}
    });
    let path = home.join("settings.json");
    std::fs::write(&path, serde_json::to_string_pretty(&value).expect("json")).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
}

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
            daemon: Daemon::default(),
        },
    )
    .expect("init");
}

#[test]
fn restore_rebuilds_the_post_onboarding_layout() {
    let home = TempDir::new().expect("temp home");
    write_settings(home.path());

    let restore = restore_home(home.path()).expect("restore");

    for directory in LAYOUT {
        assert!(
            home.path().join(directory).is_dir(),
            "missing directory {directory}"
        );
    }
    let soul = std::fs::read_to_string(home.path().join("SOUL.md")).expect("SOUL.md");
    assert!(soul.contains("<identity>"), "soul was: {soul}");
    assert!(soul.contains("Hearth"), "soul was: {soul}");
    assert!(
        std::fs::read_to_string(home.path().join("HEARTH.md"))
            .expect("HEARTH.md")
            .is_empty()
    );
    assert_eq!(restore.created.len(), 2, "created: {:?}", restore.created);
    // The database belongs to the daemon's writer, never to the repair.
    assert!(
        !home.path().join("hearth.db").exists(),
        "restore created the database"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["SOUL.md", "HEARTH.md"] {
            let mode = std::fs::metadata(home.path().join(name))
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name} mode was {mode:o}");
        }
    }
}

#[test]
fn restore_keeps_files_it_did_not_create() {
    let home = TempDir::new().expect("temp home");
    write_settings(home.path());
    std::fs::write(home.path().join("SOUL.md"), "custom soul\n").expect("soul");
    std::fs::write(home.path().join("HEARTH.md"), "custom guidance\n").expect("guidance");

    let restore = restore_home(home.path()).expect("restore");

    assert!(restore.created.is_empty(), "created: {:?}", restore.created);
    assert_eq!(
        std::fs::read_to_string(home.path().join("SOUL.md")).expect("soul"),
        "custom soul\n"
    );
    assert_eq!(
        std::fs::read_to_string(home.path().join("HEARTH.md")).expect("guidance"),
        "custom guidance\n"
    );
}

#[test]
fn restore_is_quiet_on_a_fresh_init_home() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());

    let restore = restore_home(home.path()).expect("restore");

    assert!(restore.created.is_empty(), "created: {:?}", restore.created);
}

#[test]
fn restore_refuses_a_home_without_settings() {
    let home = TempDir::new().expect("temp home");

    let error = restore_home(home.path()).expect_err("missing settings");

    assert!(error.to_string().contains("settings.json"), "{error}");
    assert!(
        !home.path().join("workspace").exists(),
        "restore wrote a layout without settings"
    );
}
