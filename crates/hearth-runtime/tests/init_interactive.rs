use std::io::Cursor;

use hearth_runtime::{InitOptions, init_with_io};
use tempfile::TempDir;

#[test]
fn interactive_init_accepts_defaults_and_never_echoes_secrets() {
    let home = TempDir::new().expect("temp home");
    let input = "fake-token\n123\nhttps://example.com/v1\nfake-key\nfake-model\n\n\nUTC\nn\nn\nn\n";
    let mut reader = Cursor::new(input.as_bytes());
    let mut output = Vec::new();
    init_with_io(
        home.path(),
        InitOptions::default(),
        &mut reader,
        &mut output,
    )
    .expect("interactive init");

    let settings = hearth_config::load(home.path()).expect("load settings");
    let telegram = settings.telegram.as_ref().expect("telegram settings");
    assert_eq!(telegram.bot_token.as_deref(), Some("fake-token"));
    assert_eq!(telegram.allowed_user_ids, vec![123]);
    let models = settings.models.as_ref().expect("models");
    assert_eq!(models.model, "fake-model");
    assert_eq!(settings.identity.agent_name, "Hearth");
    assert_eq!(settings.identity.user_name, "User");
    assert_eq!(settings.identity.timezone, "UTC");
    assert!(settings.media.is_none());

    let text = String::from_utf8(output).expect("utf-8 output");
    assert!(!text.contains("fake-token"), "token echoed: {text}");
    assert!(!text.contains("fake-key"), "key echoed: {text}");
    assert!(text.contains("created"), "output: {text}");
    assert!(home.path().join("hearth.db").is_file());
}
