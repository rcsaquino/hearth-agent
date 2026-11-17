use hearth_config::Settings;
use serde_json::json;

fn parse(value: serde_json::Value) -> Settings {
    serde_json::from_value(value).expect("settings")
}

#[test]
fn the_eta_reminder_defaults_to_one_second() {
    let settings = parse(json!({}));
    assert_eq!(settings.tasks.eta_reminder_seconds, 1);
}

#[test]
fn the_eta_reminder_accepts_seconds() {
    let settings = parse(json!({"tasks": {"eta_reminder_seconds": 0}}));
    assert_eq!(settings.tasks.eta_reminder_seconds, 0);
}

#[test]
fn the_old_minutes_setting_is_rejected() {
    let error = serde_json::from_value::<Settings>(json!({
        "tasks": {"eta_reminder_minutes": 10}
    }))
    .expect_err("a removed key is a schema error");
    assert!(
        error.to_string().contains("eta_reminder_minutes"),
        "{error}"
    );
}
