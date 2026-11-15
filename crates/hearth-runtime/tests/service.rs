use std::fs;

use hearth_runtime::service::{
    ServiceError, apply_start_mode_at, install_at, repair_at, status_at, uninstall_at,
};
use tempfile::TempDir;

#[test]
fn install_writes_a_unit_and_is_idempotent() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    let first = install_at(home.path(), config.path(), false, false).expect("install");
    let text = fs::read_to_string(&first.unit).expect("read unit");
    assert!(text.contains("[Service]"), "unit is {text}");
    assert!(text.contains("ExecStart="));
    assert!(text.contains("Environment=\"HEARTH_DAEMON=1\""));
    assert!(
        text.contains("Environment=\"HEARTH_HOME="),
        "unit is {text}"
    );
    assert!(
        !text.contains("--home"),
        "the unit must not use the removed CLI flag: {text}"
    );
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
    let missing = status_at(config.path(), home.path()).expect("status");
    assert!(!missing.installed);

    install_at(home.path(), config.path(), false, false).expect("install");
    let installed = status_at(config.path(), home.path()).expect("status");
    assert!(installed.installed);
    assert_eq!(installed.unit, config.path().join("hearth.service"));
}

#[test]
fn start_modes_write_the_env_unit_and_report_linger() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");

    let login =
        apply_start_mode_at(home.path(), config.path(), "login", false).expect("login mode");
    assert!(login.installed);
    assert!(!login.linger);
    let text = fs::read_to_string(&login.unit).expect("unit");
    assert!(text.contains("Environment=\"HEARTH_DAEMON=1\""));
    assert!(text.contains("Environment=\"HEARTH_HOME="));
    assert!(!text.contains("--home"));

    let startup =
        apply_start_mode_at(home.path(), config.path(), "startup", false).expect("startup mode");
    assert!(startup.installed);
    assert!(startup.linger);
    assert!(startup.unit.is_file());

    let tui = apply_start_mode_at(home.path(), config.path(), "tui", false).expect("tui mode");
    assert!(!tui.installed);
    assert!(!tui.linger);
    assert!(!tui.unit.exists());

    let error = apply_start_mode_at(home.path(), config.path(), "sometimes", false)
        .expect_err("unknown mode");
    assert!(matches!(error, ServiceError::Config(_)));
}

#[test]
fn uninstall_removes_the_unit_and_is_idempotent() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    install_at(home.path(), config.path(), false, false).expect("install");

    let removed = uninstall_at(config.path(), false).expect("uninstall");
    assert!(removed.removed);
    assert_eq!(removed.unit, config.path().join("hearth.service"));
    assert!(!removed.unit.exists());
    assert!(
        !status_at(config.path(), home.path())
            .expect("status")
            .installed
    );

    let again = uninstall_at(config.path(), false).expect("idempotent uninstall");
    assert!(!again.removed);
}

#[test]
fn a_unit_that_names_another_home_is_not_installed_for_this_one() {
    let ours = TempDir::new().expect("temp home");
    let theirs = TempDir::new().expect("other home");
    let config = TempDir::new().expect("systemd config");
    install_at(theirs.path(), config.path(), false, false).expect("install");

    assert!(
        status_at(config.path(), theirs.path())
            .expect("status")
            .installed,
        "the unit names its own home"
    );
    assert!(
        !status_at(config.path(), ours.path())
            .expect("status")
            .installed,
        "a unit for another home must not manage this one"
    );
}

fn enable_link(config: &std::path::Path) -> std::path::PathBuf {
    config.join("default.target.wants").join("hearth.service")
}

#[test]
fn install_writes_the_enable_link() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    let report = install_at(home.path(), config.path(), false, false).expect("install");

    let link = enable_link(config.path());
    assert!(link.exists(), "the enable link is missing");
    assert_eq!(fs::read_link(&link).expect("read link"), report.unit);
    assert_eq!(report.unit, config.path().join("hearth.service"));
}

#[test]
fn reapplying_a_start_mode_recreates_a_deleted_unit_and_link() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    apply_start_mode_at(home.path(), config.path(), "login", false).expect("login mode");

    let unit = config.path().join("hearth.service");
    let link = enable_link(config.path());
    fs::remove_file(&unit).expect("remove unit");
    fs::remove_file(&link).expect("remove link");

    apply_start_mode_at(home.path(), config.path(), "login", false).expect("reapply");
    assert!(unit.is_file(), "the unit was not recreated");
    assert!(link.exists(), "the enable link was not recreated");
    assert_eq!(fs::read_link(&link).expect("read link"), unit);
}

#[test]
fn uninstall_removes_the_enable_link() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    install_at(home.path(), config.path(), false, false).expect("install");

    let link = enable_link(config.path());
    assert!(link.exists());
    uninstall_at(config.path(), false).expect("uninstall");
    assert!(!config.path().join("hearth.service").exists());
    assert!(!link.exists(), "the enable link survived uninstall");
}

fn write_start_settings(home: &std::path::Path, start: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = home.join("settings.json");
    fs::write(&path, format!("{{\"daemon\":{{\"start\":\"{start}\"}}}}")).expect("settings");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("settings mode");
}

#[test]
fn repairing_creates_a_missing_unit_and_link() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    write_start_settings(home.path(), "login");

    let report = repair_at(home.path(), config.path(), false)
        .expect("repair")
        .expect("login report");
    assert!(
        report.repaired,
        "a missing unit must be reported as repaired"
    );
    assert_eq!(report.start, "login");

    let unit = config.path().join("hearth.service");
    let link = enable_link(config.path());
    assert_eq!(report.unit, unit);
    assert!(unit.is_file(), "the unit was not created");
    assert!(link.exists(), "the enable link was not created");
    assert_eq!(fs::read_link(&link).expect("read link"), unit);
}

#[test]
fn repairing_restores_a_deleted_link_without_rewriting_the_unit() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    write_start_settings(home.path(), "login");
    let installed = install_at(home.path(), config.path(), false, false).expect("install");
    let unit = installed.unit;
    let bytes = fs::read(&unit).expect("unit bytes");
    let modified = fs::metadata(&unit)
        .expect("unit metadata")
        .modified()
        .expect("mtime");

    let link = enable_link(config.path());
    fs::remove_file(&link).expect("remove link");
    let report = repair_at(home.path(), config.path(), false)
        .expect("repair")
        .expect("login report");
    assert!(
        report.repaired,
        "a missing link must be reported as repaired"
    );
    assert_eq!(fs::read_link(&link).expect("read link"), unit);
    assert_eq!(fs::read(&unit).expect("unit bytes"), bytes);
    assert_eq!(
        fs::metadata(&unit)
            .expect("unit metadata")
            .modified()
            .expect("mtime"),
        modified,
        "repair rewrote an existing unit"
    );
}

#[test]
fn repairing_does_nothing_for_the_tui_mode() {
    let home = TempDir::new().expect("temp home");
    let config = TempDir::new().expect("systemd config");
    write_start_settings(home.path(), "tui");
    let installed = install_at(home.path(), config.path(), false, false).expect("install");
    let unit = installed.unit;
    let link = enable_link(config.path());
    let bytes = fs::read(&unit).expect("unit bytes");
    let modified = fs::metadata(&unit)
        .expect("unit metadata")
        .modified()
        .expect("mtime");
    let target = fs::read_link(&link).expect("read link");

    let report = repair_at(home.path(), config.path(), false).expect("repair");
    assert!(
        report.is_none(),
        "tui mode must not report a service change"
    );
    assert_eq!(fs::read(&unit).expect("unit bytes"), bytes);
    assert_eq!(
        fs::metadata(&unit)
            .expect("unit metadata")
            .modified()
            .expect("mtime"),
        modified
    );
    assert_eq!(fs::read_link(&link).expect("read link"), target);
}

#[test]
fn repairing_leaves_a_unit_that_names_another_home_alone() {
    let home = TempDir::new().expect("temp home");
    let other = TempDir::new().expect("other home");
    let config = TempDir::new().expect("systemd config");
    write_start_settings(home.path(), "login");

    let unit = config.path().join("hearth.service");
    let content = format!(
        "[Service]\nEnvironment=\"HEARTH_HOME={}\"\n",
        other.path().display()
    );
    fs::write(&unit, &content).expect("other unit");
    let report = repair_at(home.path(), config.path(), false).expect("repair");
    assert!(
        report.is_none(),
        "a unit for another home must be left alone"
    );
    assert_eq!(fs::read_to_string(&unit).expect("unit"), content);
    assert!(!enable_link(config.path()).exists());
}
