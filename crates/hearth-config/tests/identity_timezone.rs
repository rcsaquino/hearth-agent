use std::fs;
use std::path::Path;

use hearth_config::identity_timezone;
use tempfile::TempDir;

fn write(home: &Path, text: &str) {
    let path = home.join("settings.json");
    fs::write(&path, text).expect("write settings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("settings mode");
    }
}

#[test]
fn the_configured_zone_is_read() {
    let home = TempDir::new().expect("home");
    write(
        home.path(),
        r#"{"identity":{"agent_name":"Hearth","timezone":"Asia/Manila"}}"#,
    );
    assert_eq!(identity_timezone(home.path()), "Asia/Manila");
}

#[test]
fn a_missing_file_falls_back_to_utc() {
    let home = TempDir::new().expect("home");
    assert_eq!(identity_timezone(home.path()), "UTC");
}

#[test]
fn invalid_json_falls_back_to_utc() {
    let home = TempDir::new().expect("home");
    write(home.path(), "{not json");
    assert_eq!(identity_timezone(home.path()), "UTC");
}

#[test]
fn a_missing_empty_or_malformed_key_falls_back_to_utc() {
    for text in [
        r#"{"identity":{}}"#,
        r#"{"identity":{"timezone":""}}"#,
        r#"{"identity":"UTC"}"#,
    ] {
        let home = TempDir::new().expect("home");
        write(home.path(), text);
        assert_eq!(
            identity_timezone(home.path()),
            "UTC",
            "settings were {text}"
        );
    }
}
