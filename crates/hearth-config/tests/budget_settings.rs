use hearth_config::Settings;

fn models() -> String {
    r#"{"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m"}}"#
        .to_owned()
}

#[test]
fn budget_settings_have_the_spec_defaults() {
    let settings: Settings = serde_json::from_str(&models()).expect("settings");
    settings.validate().expect("valid settings");
    let models = settings.models.expect("models");
    assert_eq!(models.context_window, 128_000);
    assert_eq!(models.max_output_tokens, 32_000);
    assert!((settings.compaction.soft_compact - 0.65).abs() < f64::EPSILON);
    assert!((settings.compaction.hard_compact - 0.85).abs() < f64::EPSILON);
    assert_eq!(settings.compaction.keep_recent_tokens, 20_000);
}

#[test]
fn budget_settings_accept_a_custom_window() {
    let settings: Settings = serde_json::from_str(
        r#"{
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake",
                "model": "m",
                "context_window": 64000,
                "max_output_tokens": 4096
            },
            "compaction": {
                "soft_compact": 0.5,
                "hard_compact": 0.8,
                "keep_recent_tokens": 10000
            }
        }"#,
    )
    .expect("settings");
    settings.validate().expect("valid settings");
    let models = settings.models.expect("models");
    assert_eq!(models.context_window, 64_000);
    assert_eq!(models.max_output_tokens, 4_096);
    assert_eq!(settings.compaction.keep_recent_tokens, 10_000);
}

#[test]
fn agent_has_no_turn_limit_setting() {
    let settings: Settings = serde_json::from_str(&models()).expect("settings");
    assert_eq!(settings.agent.workspace, None);
}

#[test]
fn thinking_defaults_to_the_provider_choice() {
    let settings: Settings = serde_json::from_str(&models()).expect("settings");
    settings.validate().expect("valid settings");
    assert!(settings.models.expect("models").thinking.is_none());
}

#[test]
fn thinking_accepts_a_level_and_rejects_an_unknown_value() {
    let accepted: Settings = serde_json::from_str(
        r#"{"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m","thinking":"low"}}"#,
    )
    .expect("settings");
    accepted.validate().expect("valid settings");
    assert_eq!(
        accepted.models.expect("models").thinking.as_deref(),
        Some("low")
    );

    let rejected: Settings = serde_json::from_str(
        r#"{"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m","thinking":"sometimes"}}"#,
    )
    .expect("settings");
    let error = rejected.validate().expect_err("unknown thinking");
    assert!(error.to_string().contains("models.thinking"), "{error}");
}

#[test]
fn a_zero_context_window_is_rejected() {
    let settings: Settings = serde_json::from_str(
        r#"{"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m","context_window":0}}"#,
    )
    .expect("settings");
    let error = settings.validate().expect_err("zero window");
    assert!(
        error.to_string().contains("models.context_window"),
        "{error}"
    );
}

#[test]
fn soft_compaction_must_be_below_hard() {
    let settings: Settings =
        serde_json::from_str(r#"{"compaction":{"soft_compact":0.9,"hard_compact":0.8}}"#)
            .expect("settings");
    let error = settings.validate().expect_err("bad thresholds");
    assert!(
        error.to_string().contains("compaction.soft_compact"),
        "{error}"
    );
}

#[test]
fn a_zero_keep_recent_tokens_is_rejected() {
    let settings: Settings =
        serde_json::from_str(r#"{"compaction":{"keep_recent_tokens":0}}"#).expect("settings");
    let error = settings.validate().expect_err("zero keep");
    assert!(
        error.to_string().contains("compaction.keep_recent_tokens"),
        "{error}"
    );
}

#[test]
fn thinking_accepts_xhigh_and_max() {
    for level in ["xhigh", "max"] {
        let settings: Settings = serde_json::from_str(&format!(
            r#"{{"models":{{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m","thinking":"{level}"}}}}"#,
        ))
        .expect("settings");
        settings.validate().expect("valid thinking level");
        assert_eq!(
            settings.models.expect("models").thinking.as_deref(),
            Some(level)
        );
    }
}

#[test]
fn a_legacy_soul_file_setting_is_ignored() {
    let home = tempfile::TempDir::new().expect("home");
    let path = home.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"identity":{"agent_name":"Hearth","timezone":"UTC","soul_file":"ALT.md"},"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m"}}"#,
    )
    .expect("settings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
    let settings = hearth_config::load(home.path()).expect("load");
    assert_eq!(settings.identity.agent_name, "Hearth");
}
