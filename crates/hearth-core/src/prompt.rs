//! Prompt assembly from the permanent record.

use std::collections::BTreeMap;

use hearth_model::{ChatMessage, ChatRequest, ContentBlock, Role, ToolSpec};
use serde_json::{Value, json};

use crate::budget::TokenCounts;
use crate::error::CoreError;
use crate::model::Entry;
use crate::session::Session;
use uuid::Uuid;

/// The operation layer, kept short and stable.
pub const OPERATION: &str = "## Operation\n\n\
You act for one user on one local machine. Tools run without approval prompts. \
Use tools when they help, keep answers plain and brief, and never reveal secrets. \
Use the message tool to send files, and use voice only when the user explicitly \
asks for voice.";

/// Inputs for one assembled prompt.
#[derive(Debug, Clone)]
pub struct PromptInput {
    pub soul: String,
    pub conversation_id: String,
    pub model: String,
    pub tools: Vec<ToolSpec>,
    pub standing: String,
    pub standing_version: u64,
    pub recall: String,
    pub runtime_note: String,
}

/// One assembled prompt and its audit view.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub request: ChatRequest,
    pub document: Value,
    pub tokens: TokenCounts,
    pub prefix_hash: String,
    pub prefix_parts: BTreeMap<String, String>,
}

/// Assemble the request and the audit document.
pub fn build(input: PromptInput, history: &[Entry]) -> Result<Prompt, CoreError> {
    let mut messages = Vec::with_capacity(history.len() + 4);
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(format!(
            "{}\n\n{}",
            input.soul, OPERATION
        ))],
    });
    if !input.standing.trim().is_empty() {
        messages.push(ChatMessage {
            role: Role::System,
            blocks: vec![ContentBlock::Text(format!(
                "## Standing memory\n\n{}",
                input.standing
            ))],
        });
    }
    let newest_user = history.iter().rposition(|entry| {
        message_for_entry(entry).is_some_and(|message| message.role == Role::User)
    });
    for (index, entry) in history.iter().enumerate() {
        if Some(index) == newest_user && !input.recall.trim().is_empty() {
            messages.push(ChatMessage {
                role: Role::System,
                blocks: vec![ContentBlock::Text(format!(
                    "## Relevant memory\n\n{}",
                    input.recall
                ))],
            });
        }
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
    let tools_text = if tools.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&tools).map_err(|error| CoreError::Invalid(error.to_string()))?
    };
    let transcript_text = serde_json::to_string(&transcript)
        .map_err(|error| CoreError::Invalid(error.to_string()))?;
    let tokens = TokenCounts::new(
        tokens_for(&input.soul),
        tokens_for(OPERATION),
        tokens_for(&tools_text),
        tokens_for(&input.standing),
        0,
        tokens_for(&transcript_text),
        tokens_for(&input.recall),
        tokens_for(&input.runtime_note),
    );
    let skills_text = String::new();
    let mut prefix_parts = BTreeMap::new();
    prefix_parts.insert("identity".to_owned(), hash_part(&input.soul));
    prefix_parts.insert("operation".to_owned(), hash_part(OPERATION));
    prefix_parts.insert("tools".to_owned(), hash_part(&tools_text));
    prefix_parts.insert(
        "standing".to_owned(),
        hash_part(&format!("{}\n{}", input.standing, input.standing_version)),
    );
    prefix_parts.insert("skills".to_owned(), hash_part(&skills_text));
    let prefix_hash = hash_part(
        &prefix_parts
            .iter()
            .map(|(name, hash)| format!("{name}:{hash}"))
            .collect::<Vec<_>>()
            .join("|"),
    );
    let document = json!({
        "identity": input.soul,
        "operation": OPERATION,
        "tools": tools,
        "standing_memory": input.standing,
        "recall": input.recall,
        "transcript": transcript,
        "runtime": input.runtime_note,
        "model": input.model,
        "session_id": input.conversation_id,
        "prefix_hash": prefix_hash,
        "prefix_parts": prefix_parts,
        "tokens": {
            "identity": tokens.identity,
            "operation": tokens.operation,
            "tools": tokens.tools,
            "standing": tokens.standing,
            "skills": tokens.skills,
            "transcript": tokens.transcript,
            "recall": tokens.recall,
            "runtime": tokens.runtime,
            "total": tokens.total,
            "fixed": tokens.fixed,
        },
    });

    let request = ChatRequest {
        model: input.model,
        messages,
        tools: input.tools,
        temperature: None,
        max_tokens: None,
        session_id: Some(input.conversation_id),
    };

    Ok(Prompt {
        request,
        document,
        tokens,
        prefix_hash,
        prefix_parts,
    })
}

fn hash_part(text: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, text.as_bytes()).to_string()
}

/// Compare two prefix part maps and name the layers that changed.
pub fn prefix_cause(
    previous: Option<&Value>,
    current: &BTreeMap<String, String>,
) -> Vec<String> {
    let Some(previous) = previous.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut causes = Vec::new();
    for (part, hash) in current {
        let changed = previous.get(part).and_then(Value::as_str) != Some(hash.as_str());
        if !changed {
            continue;
        }
        let label = match part.as_str() {
            "identity" => "SOUL.md",
            "operation" => "operation",
            "tools" => "tools",
            "standing" => "standing memory",
            "skills" => "skills index",
            _ => part.as_str(),
        };
        causes.push(label.to_owned());
    }
    causes
}

fn tokens_for(text: &str) -> u64 {
    hearth_model::estimate_tokens(text).max(0) as u64
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
        ContentBlock::Image { path, mime } => {
            json!({"type": "image", "path": path, "mime": mime})
        }
        ContentBlock::File { path, name, mime } => json!({
            "type": "file",
            "path": path,
            "name": name,
            "mime": mime,
        }),
    }
}

/// Map one permanent entry to a chat message.
pub fn message_for_entry(entry: &Entry) -> Option<ChatMessage> {
    if entry.kind == "summary" {
        let text = crate::compaction::summary_text(entry)?;
        return Some(ChatMessage {
            role: Role::System,
            blocks: vec![ContentBlock::Text(format!(
                "## Summary of earlier conversation\n\n{text}"
            ))],
        });
    }
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
            arguments: object
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({})),
        })),
        Some("tool_result") => Some(ContentBlock::ToolResult {
            call_id: object.get("call_id")?.as_str()?.to_owned(),
            content: object
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
        Some("image") => Some(ContentBlock::Image {
            path: object.get("path")?.as_str()?.to_owned(),
            mime: object
                .get("mime")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_owned(),
        }),
        Some("file") => Some(ContentBlock::File {
            path: object.get("path")?.as_str()?.to_owned(),
            name: object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("file")
                .to_owned(),
            mime: object
                .get("mime")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_owned(),
        }),
        Some(_) => None,
        None => Some(ContentBlock::Text(text_field(object)?)),
    }
}

fn text_field(object: &serde_json::Map<String, Value>) -> Option<String> {
    object
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
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
            ContentBlock::Image { path, .. } => path.clone(),
            ContentBlock::File { name, .. } => name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    hearth_model::estimate_tokens(&text)
}

/// Build a prompt for a conversation.
pub fn build_for(session: &Session, input: PromptInput) -> Result<Prompt, CoreError> {
    let history = session.history_for_prompt(&input.conversation_id)?;
    build(input, &history)
}

/// Rebuild the provider-neutral request from a saved prompt audit document.
pub fn request_from_document(document: &Value) -> Option<ChatRequest> {
    let model = document.get("model")?.as_str()?.to_owned();
    let conversation_id = document.get("session_id")?.as_str()?.to_owned();
    let identity = document.get("identity")?.as_str()?;
    let operation = document.get("operation")?.as_str()?;
    let standing = document
        .get("standing_memory")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let recall = document
        .get("recall")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let runtime = document
        .get("runtime")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tools = document
        .get("tools")?
        .as_array()?
        .iter()
        .map(|tool| {
            Some(ToolSpec {
                name: tool.get("name")?.as_str()?.to_owned(),
                description: tool.get("description")?.as_str()?.to_owned(),
                parameters: tool.get("parameters")?.clone(),
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let mut messages = vec![ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(format!("{identity}\n\n{operation}"))],
    }];
    if !standing.trim().is_empty() {
        messages.push(ChatMessage {
            role: Role::System,
            blocks: vec![ContentBlock::Text(format!(
                "## Standing memory\n\n{standing}"
            ))],
        });
    }
    let transcript = document.get("transcript")?.as_array()?;
    let parsed: Vec<ChatMessage> = transcript
        .iter()
        .filter_map(|message| {
            let role = match message.get("role")?.as_str()? {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => return None,
            };
            let blocks = message
                .get("blocks")?
                .as_array()?
                .iter()
                .flat_map(parse_blocks)
                .collect();
            Some(ChatMessage { role, blocks })
        })
        .collect();
    let newest_user = parsed.iter().rposition(|message| message.role == Role::User);
    for (index, message) in parsed.into_iter().enumerate() {
        if Some(index) == newest_user && !recall.trim().is_empty() {
            messages.push(ChatMessage {
                role: Role::System,
                blocks: vec![ContentBlock::Text(format!(
                    "## Relevant memory\n\n{recall}"
                ))],
            });
        }
        messages.push(message);
    }
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(runtime.to_owned())],
    });

    Some(ChatRequest {
        model,
        messages,
        tools,
        temperature: None,
        max_tokens: document
            .get("max_tokens")
            .and_then(Value::as_u64)
            .map(|value| value.min(u32::MAX as u64) as u32),
        session_id: Some(conversation_id),
    })
}
