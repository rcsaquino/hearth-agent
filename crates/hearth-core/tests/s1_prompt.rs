use hearth_core::{Harness, PromptInput, Save};
use hearth_model::Role;
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

fn text_of(message: &hearth_model::ChatMessage) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            hearth_model::ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_prompt_starts_with_identity_and_operation_and_ends_with_runtime() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    session
        .save(Save::new(&conversation.id).entry("user", json!({"text": "hello"})))
        .expect("save user entry");

    let prompt = session
        .build_prompt(PromptInput {
            soul: "SOUL TEXT".to_owned(),
            conversation_id: conversation.id.clone(),
            model: "test-model".to_owned(),
            tools: Vec::new(),
            runtime_note: "RUNTIME NOTE".to_owned(),
        })
        .expect("build prompt");

    let first = prompt.request.messages.first().expect("first message");
    assert_eq!(first.role, Role::System);
    let first_text = text_of(first);
    assert!(first_text.contains("SOUL TEXT"), "missing soul: {first_text}");
    assert!(
        first_text.contains("Operation"),
        "missing operation: {first_text}"
    );

    let last = prompt.request.messages.last().expect("last message");
    assert_eq!(last.role, Role::System);
    assert_eq!(text_of(last), "RUNTIME NOTE");

    let user = prompt
        .request
        .messages
        .iter()
        .find(|message| message.role == Role::User)
        .expect("user message");
    assert_eq!(text_of(user), "hello");
}
