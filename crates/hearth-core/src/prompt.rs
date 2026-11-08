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

    let transcript: Vec<Value> = history
        .iter()
        .filter_map(message_for_entry)
        .map(|message| {
            json!({
                "role": role_str(message.role),
                "blocks": message.blocks.iter().map(block_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    let tools: Vec<Value> = input
        .tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect();
    let document = json!({
        "identity": input.soul,
        "operation": OPERATION,
        "tools": tools,
        "transcript": transcript,
        "runtime": input.runtime_note,
    });

    let request = ChatRequest {
        model: input.model,
        messages,
        tools: input.tools,
        temperature: None,
        max_tokens: None,
        session_id: Some(input.conversation_id),
    };

    Ok(Prompt { request, document })
}

fn role_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn block_json(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text(text) => json!({"type": "text", "text": text}),
        ContentBlock::Thinking(text) => json!({"type": "thinking", "text": text}),
        ContentBlock::ToolCall(call) => json!({
            "type": "tool_call",
            "id": call.id,
            "name": call.name,
            "arguments": call.arguments,
        }),
        ContentBlock::ToolResult { call_id, content } => json!({
            "type": "tool_result",
            "call_id": call_id,
            "content": content,
        }),
    }
}

/// Map one permanent entry to a chat message.
pub fn message_for_entry(entry: &Entry) -> Option<ChatMessage> {
    let role = match entry.kind.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool_result" => Role::Tool,
        "system" | "notice" => Role::System,
        _ => return None,
    };
    let blocks = parse_blocks(&entry.content)
        .into_iter()
        .filter(|block| !matches!(block, ContentBlock::Thinking(_)))
        .collect();
    Some(ChatMessage { role, blocks })
}

/// Parse entry content into provider-neutral blocks.
pub fn parse_blocks(value: &Value) -> Vec<ContentBlock> {
    match value {
        Value::String(text) => vec![ContentBlock::Text(text.clone())],
        Value::Object(object) => parse_block(object).into_iter().collect(),
        Value::Array(items) => items.iter().flat_map(parse_blocks).collect(),
        _ => Vec::new(),
    }
}

fn parse_block(object: &serde_json::Map<String, Value>) -> Option<ContentBlock> {
    let block_type = object.get("type").and_then(Value::as_str);
    match block_type {
        Some("text") => Some(ContentBlock::Text(text_field(object)?)),
        Some("thinking") => Some(ContentBlock::Thinking(text_field(object)?)),
        Some("tool_call") => Some(ContentBlock::ToolCall(hearth_model::ToolCall {
            id: object.get("id")?.as_str()?.to_owned(),
            name: object.get("name")?.as_str()?.to_owned(),
            arguments: object.get("arguments").cloned().unwrap_or_else(|| json!({})),
        })),
        Some("tool_result") => Some(ContentBlock::ToolResult {
            call_id: object.get("call_id")?.as_str()?.to_owned(),
            content: object
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
        Some(_) => None,
        None => Some(ContentBlock::Text(text_field(object)?)),
    }
}

fn text_field(object: &serde_json::Map<String, Value>) -> Option<String> {
    object.get("text").and_then(Value::as_str).map(str::to_owned)
}

/// Estimate the tokens an entry contributes.
pub fn estimate_content(value: &Value) -> i64 {
    let blocks = parse_blocks(value);
    if blocks.is_empty() {
        return hearth_model::estimate_tokens(&value.to_string());
    }
    let text = blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text(text) | ContentBlock::Thinking(text) => text.clone(),
            ContentBlock::ToolCall(call) => format!("{} {}", call.name, call.arguments),
            ContentBlock::ToolResult { content, .. } => content.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    hearth_model::estimate_tokens(&text)
}

/// Build a prompt for a conversation.
pub fn build_for(session: &Session, input: PromptInput) -> Result<Prompt, CoreError> {
    let history = session.history(&input.conversation_id)?;
    build(input, &history)
}
