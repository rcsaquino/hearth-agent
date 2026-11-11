use hearth_config::{Daemon, load};
use hearth_runtime::{InitOptions, init};
use serde_json::json;
use tempfile::TempDir;

fn options() -> InitOptions {
    InitOptions {
        telegram_token: "123:fake-token".to_owned(),
        allowed_user_ids: vec![42],
        base_url: "https://api.deepseek.com/v1".to_owned(),
        api_key: "fake-key".to_owned(),
        model: "deepseek-flash".to_owned(),
        thinking: "high".to_owned(),
        agent_name: "Hearth".to_owned(),
        timezone: Some("UTC".to_owned()),
        media: None,
        context_window: None,
        daemon: Daemon::default(),
    }
}

#[test]
fn init_writes_a_complete_home_silently() {
    let home = TempDir::new().expect("temp home");
    let report = init(
        home.path(),
        InitOptions {
            media: Some(json!({
                "image": {
                    "kind": "openrouter-images",
                    "api_key": "fake-image-key",
                    "model": "openai/gpt-image-2.6-sunburst"
                }
            })),
            daemon: Daemon {
                start: "login".to_owned(),
                keep_running: false,
            },
            ..options()
        },
    )
    .expect("init");

    assert_eq!(report.home, home.path());
    assert_eq!(report.settings, home.path().join("settings.json"));
    assert_eq!(report.database, home.path().join("hearth.db"));
    assert_eq!(report.schema_version, hearth_storage::SCHEMA_VERSION);

    let settings = load(home.path()).expect("load settings");
    assert_eq!(settings.identity.agent_name, "Hearth");
    assert_eq!(settings.identity.timezone, "UTC");
    assert_eq!(settings.agent.max_turns, 120);
    let settings_text = std::fs::read_to_string(&report.settings).expect("settings text");
    assert!(
        !settings_text.contains("\"soul_file\""),
        "settings wrote a default soul file: {settings_text}"
    );
    assert_eq!(
        settings
            .telegram
            .as_ref()
            .expect("telegram")
            .allowed_user_ids,
        vec![42]
    );
    let models = settings.models.as_ref().expect("models");
    assert_eq!(models.model, "deepseek-flash");
    assert_eq!(models.thinking.as_deref(), Some("high"));
    assert_eq!(settings.daemon.start, "login");
    assert!(!settings.daemon.keep_running);
    let media = settings.media.as_ref().expect("media");
    assert_eq!(
        media.image.as_ref().expect("image").kind.as_deref(),
        Some("openrouter-images")
    );

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
    assert!(report.database.is_file());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&report.settings)
            .expect("settings metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "settings mode was {mode:o}");
    }
}

#[test]
fn init_rejects_invalid_input_before_creating_the_home() {
    let root = TempDir::new().expect("temp root");
    let home = root.path().join("new-home");
    let error = init(
        &home,
        InitOptions {
            thinking: "sometimes".to_owned(),
            ..options()
        },
    )
    .expect_err("invalid thinking");
    assert!(error.to_string().contains("thinking"), "{error}");
    assert!(!home.exists(), "init created a home before validating");

    let error = init(
        &home,
        InitOptions {
            allowed_user_ids: Vec::new(),
            ..options()
        },
    )
    .expect_err("missing user id");
    assert!(error.to_string().contains("user id"), "{error}");
    assert!(!home.exists());
}

#[test]
fn init_refuses_to_overwrite_settings() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("first init");
    let error = init(home.path(), options()).expect_err("second init");
    assert!(error.to_string().contains("already exists"), "{error}");
}
