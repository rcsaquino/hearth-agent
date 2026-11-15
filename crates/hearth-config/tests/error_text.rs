use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

use tempfile::TempDir;

#[test]
fn a_schema_error_names_the_field_without_quoting_the_value() {
    let home = TempDir::new().expect("temp home");
    let path = home.path().join("settings.json");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .expect("settings file");
    writeln!(
        file,
        r#"{{"identity":{{"agent_name":"Hearth","timezone":"UTC"}},"telegram":{{"bot_token":"123:fake","allowed_user_ids":"123456789:AAH-private-token"}}}}"#
    )
    .expect("write settings");

    let error = hearth_config::load(home.path())
        .expect_err("the wrong type is refused")
        .to_string();
    assert!(!error.contains("AAH-private-token"), "{error}");
    assert!(!error.contains("123456789"), "{error}");
    assert!(error.contains("allowed_user_ids"), "{error}");
}
