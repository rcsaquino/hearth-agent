use tempfile::TempDir;

fn write_settings(home: &TempDir, api_key: &str) {
    let settings = format!(
        r#"{{
            "identity": {{"agent_name": "Hearth", "timezone": "UTC"}},
            "models": {{
                "kind": "openai-compatible",
                "base_url": "http://127.0.0.1:1/v1",
                "api_key": "{api_key}",
                "model": "test-model"
            }},
            "logging": {{"level": "debug"}}
        }}"#
    );
    let path = home.path().join("settings.json");
    std::fs::write(&path, settings).expect("settings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("settings mode");
    }
}

#[test]
fn the_daemon_log_redacts_shape_and_configured_credentials() {
    let home = TempDir::new().expect("temp home");
    write_settings(&home, "customsecret123456");
    hearth_runtime::init_logging(home.path()).expect("init logging");
    tracing::warn!(value = "sk-test-not-a-real-key", "provider said something");
    tracing::warn!(value = "customsecret123456", "the configured key leaked");
    tracing::debug!("debug-only diagnostic");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let log = std::fs::read_to_string(home.path().join("logs/hearth.log")).expect("read log");
    assert!(!log.contains("sk-test-not-a-real-key"), "log leaked: {log}");
    assert!(
        !log.contains("customsecret123456"),
        "the configured value reached the log: {log}"
    );
    assert!(log.contains("[redacted]"));
    assert!(
        log.contains("debug-only diagnostic"),
        "logging.level must control the subscriber: {log}"
    );
}
