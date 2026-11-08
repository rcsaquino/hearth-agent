use hearth_core::Harness;
use tempfile::TempDir;

#[test]
fn conversations_are_found_by_transport_key() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    assert!(
        session
            .conversation_by_transport("telegram", "1:0")
            .expect("lookup")
            .is_none()
    );

    let conversation = session
        .create_conversation("Chat", "telegram", "1:0")
        .expect("create conversation");
    let found = session
        .conversation_by_transport("telegram", "1:0")
        .expect("lookup")
        .expect("conversation exists");
    assert_eq!(found.id, conversation.id);
    assert_eq!(found.transport, "telegram");
    assert_eq!(found.transport_key, "1:0");
    assert!(
        session
            .conversation_by_transport("telegram", "2:0")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn the_telegram_offset_defaults_to_zero_and_upserts() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    assert_eq!(session.telegram_offset().expect("offset"), 0);
    session.set_telegram_offset(5).expect("set offset");
    assert_eq!(session.telegram_offset().expect("offset"), 5);
    session.set_telegram_offset(6).expect("set offset");
    assert_eq!(session.telegram_offset().expect("offset"), 6);
}
