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
    assert!(
        first_text.contains("SOUL TEXT"),
        "missing soul: {first_text}"
    );
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

#[test]
fn assistant_tool_calls_and_tool_results_map_to_roles_and_blocks() {
    use hearth_model::{ContentBlock, ToolCall};

    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    session
        .save(
            Save::new(&conversation.id)
                .entry(
                    "user",
                    serde_json::json!([{"type": "text", "text": "read it"}]),
                )
                .entry(
                    "assistant",
                    serde_json::json!([
                        {"type": "text", "text": "calling"},
                        {"type": "thinking", "text": "private"},
                        {"type": "tool_call", "id": "call-1", "name": "read", "arguments": {"path": "a"}}
                    ]),
                )
                .entry(
                    "tool_result",
                    serde_json::json!([{"type": "tool_result", "call_id": "call-1", "content": "file body"}]),
                ),
        )
        .expect("save entries");

    let prompt = session
        .build_prompt(PromptInput {
            soul: "SOUL".to_owned(),
            conversation_id: conversation.id.clone(),
            model: "test-model".to_owned(),
            tools: Vec::new(),
            runtime_note: "RUNTIME".to_owned(),
        })
        .expect("build prompt");

    let assistant = prompt
        .request
        .messages
        .iter()
        .find(|message| message.role == Role::Assistant)
        .expect("assistant message");
    assert!(
        assistant
            .blocks
            .iter()
            .all(|block| !matches!(block, ContentBlock::Thinking(_))),
        "thinking was replayed"
    );
    assert!(
        assistant
            .blocks
            .contains(&ContentBlock::Text("calling".to_owned()))
    );
    assert!(assistant.blocks.contains(&ContentBlock::ToolCall(ToolCall {
        id: "call-1".to_owned(),
        name: "read".to_owned(),
        arguments: serde_json::json!({"path": "a"}),
    })));

    let tool = prompt
        .request
        .messages
        .iter()
        .find(|message| message.role == Role::Tool)
        .expect("tool message");
    assert_eq!(
        tool.blocks,
        vec![ContentBlock::ToolResult {
            call_id: "call-1".to_owned(),
            content: "file body".to_owned(),
        }]
    );
}

#[test]
fn the_request_carries_the_model_tools_and_session() {
    use hearth_model::ToolSpec;

    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let prompt = session
        .build_prompt(PromptInput {
            soul: "SOUL".to_owned(),
            conversation_id: conversation.id.clone(),
            model: "test-model".to_owned(),
            tools: vec![ToolSpec {
                name: "read".to_owned(),
                description: "Read a file".to_owned(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            runtime_note: "RUNTIME".to_owned(),
        })
        .expect("build prompt");

    assert_eq!(prompt.request.model, "test-model");
    assert_eq!(prompt.request.tools[0].name, "read");
    assert_eq!(
        prompt.request.session_id.as_deref(),
        Some(conversation.id.as_str())
    );
    assert_eq!(prompt.request.temperature, None);
    assert_eq!(prompt.request.max_tokens, None);
}

#[test]
fn the_audit_document_has_five_segments() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    session
        .save(Save::new(&conversation.id).entry("user", serde_json::json!({"text": "hello"})))
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

    assert_eq!(prompt.document["identity"], "SOUL TEXT");
    assert!(
        prompt.document["operation"]
            .as_str()
            .expect("operation segment")
            .contains("Operation")
    );
    assert_eq!(prompt.document["tools"], serde_json::json!([]));
    assert_eq!(
        prompt.document["transcript"][0]["role"],
        serde_json::json!("user")
    );
    assert_eq!(
        prompt.document["transcript"][0]["blocks"][0]["text"],
        serde_json::json!("hello")
    );
    assert_eq!(prompt.document["runtime"], "RUNTIME NOTE");
}

#[test]
fn saving_the_same_prompt_twice_reuses_the_hash() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let prompt = session
        .build_prompt(PromptInput {
            soul: "SOUL".to_owned(),
            conversation_id: conversation.id.clone(),
            model: "test-model".to_owned(),
            tools: Vec::new(),
            runtime_note: "RUNTIME".to_owned(),
        })
        .expect("build prompt");
    let text = serde_json::to_string(&prompt.document).expect("serialize prompt");

    let first = session.save_prompt_blob(&text).expect("save prompt");
    let second = session.save_prompt_blob(&text).expect("save prompt again");
    assert_eq!(first, second);

    let latest = session
        .latest_prompt()
        .expect("latest prompt")
        .expect("a prompt exists");
    assert_eq!(latest.hash, first);
    assert_eq!(latest.text, text);
}

#[test]
fn saved_entries_get_token_estimates() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    session
        .save(
            Save::new(&conversation.id)
                .entry("user", serde_json::json!({"text": "hello"}))
                .entry(
                    "assistant",
                    serde_json::json!([
                        {"type": "tool_call", "id": "c1", "name": "read", "arguments": {"path": "notes"}}
                    ]),
                )
                .entry("user", serde_json::json!({"text": ""})),
        )
        .expect("save entries");

    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].tokens_est, 2);
    assert!(
        history[1].tokens_est > 0,
        "tool call estimate was {}",
        history[1].tokens_est
    );
    assert_eq!(history[2].tokens_est, 0);
}

#[test]
fn image_and_file_blocks_survive_the_prompt_and_audit() {
    use hearth_model::ContentBlock;

    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    session
        .save(Save::new(&conversation.id).entry(
            "user",
            json!([
                {"type": "text", "text": "look"},
                {"type": "image", "path": "/tmp/a.png", "mime": "image/png"},
                {"type": "file", "path": "/tmp/report.pdf", "name": "report.pdf", "mime": "application/pdf"}
            ]),
        ))
        .expect("save entry");

    let prompt = session
        .build_prompt(PromptInput {
            soul: "SOUL".to_owned(),
            conversation_id: conversation.id.clone(),
            model: "test-model".to_owned(),
            tools: Vec::new(),
            runtime_note: "RUNTIME".to_owned(),
        })
        .expect("build prompt");

    let user = prompt
        .request
        .messages
        .iter()
        .find(|message| message.role == Role::User)
        .expect("user message");
    assert!(user.blocks.contains(&ContentBlock::Image {
        path: "/tmp/a.png".to_owned(),
        mime: "image/png".to_owned(),
    }));
    assert!(user.blocks.contains(&ContentBlock::File {
        path: "/tmp/report.pdf".to_owned(),
        name: "report.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
    }));
    assert_eq!(
        prompt.document["transcript"][0]["blocks"][1],
        json!({"type": "image", "path": "/tmp/a.png", "mime": "image/png"})
    );
    assert_eq!(
        prompt.document["transcript"][0]["blocks"][2],
        json!({
            "type": "file",
            "path": "/tmp/report.pdf",
            "name": "report.pdf",
            "mime": "application/pdf"
        })
    );
}

#[test]
fn the_operation_carries_the_message_and_voice_rules() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");
    let prompt = session
        .build_prompt(PromptInput {
            soul: "SOUL TEXT".to_owned(),
            conversation_id: conversation.id.clone(),
            model: "test-model".to_owned(),
            tools: Vec::new(),
            runtime_note: "RUNTIME NOTE".to_owned(),
        })
        .expect("build prompt");
    let operation = prompt.document["operation"]
        .as_str()
        .expect("operation segment");
    assert!(
        operation.contains("message tool"),
        "missing message rule: {operation}"
    );
    assert!(
        operation.contains("voice only when"),
        "missing voice rule: {operation}"
    );
}
