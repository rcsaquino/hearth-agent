use std::path::Path;

use hearth_cli::daemon_client::{self, Ownership};
use hearth_config::Daemon;
use hearth_runtime::{InitOptions, init};
use serde_json::Value;
use tempfile::TempDir;

fn init_home(home: &Path) {
    init(
        home,
        InitOptions {
            telegram_token: "123:fake-token".to_owned(),
            allowed_user_ids: vec![42],
            base_url: "https://example.com/v1".to_owned(),
            api_key: "fake-key".to_owned(),
            model: "fake-model".to_owned(),
            thinking: "default".to_owned(),
            agent_name: "Hearth".to_owned(),
            timezone: Some("UTC".to_owned()),
            media: None,
            context_window: None,
            daemon: Daemon::default(),
        },
    )
    .expect("init");
    let path = home.join("settings.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("settings")).expect("json");
    let object = value.as_object_mut().expect("settings object");
    object.remove("telegram");
    object.insert(
        "memory".to_owned(),
        serde_json::json!({
            "embeddings": true,
            "embedding_model_dir": std::env::var("HEARTH_EMBEDDING_MODEL_DIR").expect("model dir")
        }),
    );
    std::fs::write(&path, serde_json::to_string_pretty(&value).expect("json")).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
}

#[test]
#[ignore = "requires HEARTH_EMBEDDING_MODEL_DIR and a built hearth binary"]
fn the_daemon_reports_hybrid_memory_when_the_local_model_loads() {
    let home = TempDir::new().expect("temp home");
    init_home(home.path());
    let executable = Path::new(env!("CARGO_BIN_EXE_hearth"));
    let (ownership, status) =
        daemon_client::ensure_with(home.path(), "tui", executable).expect("start daemon");
    assert!(matches!(ownership, Ownership::Spawned(_)));
    assert_eq!(status["memory_recall"]["mode"], "hybrid");
    assert_eq!(status["memory_recall"]["embeddings_active"], true);
    assert_eq!(
        status["memory_recall"]["embedding_model"],
        "bekko-embedding-v1-a8m"
    );
    daemon_client::stop(home.path(), &ownership).expect("stop daemon");
}
