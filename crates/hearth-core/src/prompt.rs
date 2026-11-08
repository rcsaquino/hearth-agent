//! Prompt assembly from the permanent record.

use hearth_model::{ChatMessage, ChatRequest, ContentBlock, Role, ToolSpec};
use serde_json::{Value, json};

use crate::error::CoreError;
use crate::model::Entry;
use crate::session::Session;

/// The operation layer, kept short and stable.
pub const OPERATION: &str = "## Operation\n\n\
You act for one user on one local machine. Tools run without approval prompts. \
Use tools when they help, keep answers plain and brief, and never reveal secrets.";

/// Inputs for one assembled prompt.
#[derive(Debug, Clone)]
pub struct PromptInput {
    pub soul: String,
    pub conversation_id: String,
    pub model: String,
    pub tools: Vec<ToolSpec>,
    pub runtime_note: String,
}

/// One assembled prompt and its audit view.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub request: ChatRequest,
    pub document: Value,
}

/// Assemble the request and the audit document.
pub fn build(input: PromptInput, history: &[Entry]) -> Result<Prompt, CoreError> {
    let mut messages = Vec::with_capacity(history.len() + 2);
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(format!(
            "{}\n\n{}",
            input.soul, OPERATION
        ))],
    });
    for entry in history {
        if let Some(message) = message_for_entry(entry) {
            messages.push(message);
        }
    }
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(input.runtime_note.clone())],
    });

    let request = ChatRequest {
        model: input.model.clone(),
        messages,
        tools: input.tools,
        temperature: None,
        max_tokens: None,
        session_id: Some(input.conversation_id),
    };

    Ok(Prompt {
        request,
        document: json!({}),
    })
}

/// Map one permanent entry to a chat message.
pub fn message_for_entry(entry: &Entry) -> Option<ChatMessage> {
    let role = match entry.kind.as_str() {
        "user" => Role::User,
        _ => return None,
    };
    Some(ChatMessage {
        role,
        blocks: parse_blocks(&entry.content),
    })
}

/// Parse entry content into provider-neutral blocks.
pub fn parse_blocks(value: &Value) -> Vec<ContentBlock> {
    match value {
        Value::String(text) => vec![ContentBlock::Text(text.clone())],
        Value::Object(object) => match object.get("text").and_then(Value::as_str) {
            Some(text) => vec![ContentBlock::Text(text.to_owned())],
            None => Vec::new(),
        },
        Value::Array(items) => items.iter().flat_map(parse_blocks).collect(),
        _ => Vec::new(),
    }
}

/// Build a prompt for a conversation.
pub fn build_for(session: &Session, input: PromptInput) -> Result<Prompt, CoreError> {
    let history = session.history(&input.conversation_id)?;
    build(input, &history)
}
