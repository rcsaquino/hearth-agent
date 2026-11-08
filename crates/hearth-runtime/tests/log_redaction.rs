use tempfile::TempDir;

#[test]
fn the_daemon_log_redacts_credentials() {
    let home = TempDir::new().expect("temp home");
    hearth_runtime::init_logging(home.path()).expect("init logging");
    tracing::warn!(value = "sk-test-not-a-real-key", "provider said something");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let log = std::fs::read_to_string(home.path().join("logs/hearth.log")).expect("read log");
    assert!(!log.contains("sk-test-not-a-real-key"), "log leaked: {log}");
    assert!(log.contains("[redacted]"));
}
