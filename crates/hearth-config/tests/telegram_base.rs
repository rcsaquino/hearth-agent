use hearth_config::Settings;
use serde_json::json;

fn settings(value: serde_json::Value) -> Settings {
    serde_json::from_value(value).expect("valid settings shape")
}

fn base() -> serde_json::Value {
    json!({
        "identity": {
            "agent_name": "Hearth",
            "timezone": "UTC"
        },
        "telegram": {
            "bot_token": "123:fake",
            "allowed_user_ids": [42]
        },
        "models": {
            "kind": "openai-compatible",
            "base_url": "https://example.com/v1",
            "api_key": "fake",
            "model": "fake-model"
        }
    })
}

#[test]
fn a_custom_telegram_base_url_is_accepted() {
    let mut value = base();
    value["telegram"]["base_url"] = json!("http://127.0.0.1:8080");
    let settings = settings(value);
    assert_eq!(
        settings
            .telegram
            .as_ref()
            .and_then(|telegram| telegram.base_url.as_deref()),
        Some("http://127.0.0.1:8080")
    );
    settings.validate_for_run().expect("valid settings");
}

#[test]
fn an_invalid_telegram_base_url_is_rejected() {
    let mut value = base();
    value["telegram"]["base_url"] = json!("ftp://example.com");
    let error = settings(value)
        .validate_for_run()
        .expect_err("invalid base URL");
    assert!(
        error.to_string().contains("telegram.base_url"),
        "error was {error}"
    );
}

#[test]
fn a_missing_telegram_base_url_is_allowed() {
    let settings = settings(base());
    assert!(settings.telegram.as_ref().unwrap().base_url.is_none());
    settings.validate_for_run().expect("valid settings");
}
