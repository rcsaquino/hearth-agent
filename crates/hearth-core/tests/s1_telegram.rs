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
