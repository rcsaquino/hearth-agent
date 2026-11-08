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
    assert_eq!(models.max_output_tokens, 8_192);
    assert!((settings.compaction.soft_compact - 0.65).abs() < f64::EPSILON);
    assert!((settings.compaction.hard_compact - 0.85).abs() < f64::EPSILON);
    assert_eq!(settings.compaction.keep_recent_tokens, 24_000);
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
fn a_zero_context_window_is_rejected() {
    let settings: Settings = serde_json::from_str(
        r#"{"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m","context_window":0}}"#,
    )
    .expect("settings");
    let error = settings.validate().expect_err("zero window");
    assert!(error.to_string().contains("models.context_window"), "{error}");
}

#[test]
fn soft_compaction_must_be_below_hard() {
    let settings: Settings = serde_json::from_str(
        r#"{"compaction":{"soft_compact":0.9,"hard_compact":0.8}}"#,
    )
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
