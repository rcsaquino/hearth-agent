use std::fs;

use hearth_runtime::service::{ServiceError, install_at, status_at};
use tempfile::TempDir;

#[test]
fn install_writes_a_unit_and_is_idempotent() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    let first = install_at(home.path(), config.path(), false, false).expect("install");
    let text = fs::read_to_string(&first.unit).expect("read unit");
    assert!(text.contains("[Service]"), "unit is {text}");
    assert!(text.contains("ExecStart="));
    assert!(text.contains("daemon --home"));
    assert!(text.contains("Restart=on-failure"));
    assert!(text.contains("WantedBy=default.target"));

    let second = install_at(home.path(), config.path(), false, false).expect("idempotent");
    assert_eq!(first.unit, second.unit);
}

#[test]
fn install_refuses_a_different_unit_without_force() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    fs::write(config.path().join("hearth.service"), "a different unit").expect("write unit");
    let error = install_at(home.path(), config.path(), false, false).expect_err("refuse");
    assert!(matches!(error, ServiceError::DifferentUnit(_)));
    install_at(home.path(), config.path(), true, false).expect("force install");
}

#[test]
fn status_reports_installed_and_missing() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    let missing = status_at(config.path()).expect("status");
    assert!(!missing.installed);

    install_at(home.path(), config.path(), false, false).expect("install");
    let installed = status_at(config.path()).expect("status");
    assert!(installed.installed);
    assert_eq!(installed.unit, config.path().join("hearth.service"));
}
