//! Prompt assembly from the permanent record.

use std::collections::{BTreeMap, HashSet, VecDeque};

use hearth_model::{ChatMessage, ChatRequest, ContentBlock, Role, ToolSpec};
use serde_json::{Value, json};

use crate::budget::TokenCounts;
use crate::error::CoreError;
use crate::model::Entry;
use crate::session::Session;
use uuid::Uuid;

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
    pub skills_index: String,
    pub guidance: String,
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
        blocks: vec![ContentBlock::Text(input.soul.clone())],
    });
    // Always emit the four fixed segments, even when empty, so a transient
    // read failure cannot shift every later message by one slot.
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(format!(
            "<project_instructions>\n{}\n</project_instructions>",
            input.guidance
        ))],
    });
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(format!(
            "<standing_memory>\n{}\n</standing_memory>",
            input.standing
        ))],
    });
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(format!(
            "<skills>\n{}\n</skills>",
            input.skills_index
        ))],
    });
    let history_messages = paired_messages(history);
    let newest_user = history_messages
        .iter()
        .rposition(|message| message.role == Role::User);
    for (index, message) in history_messages.iter().enumerate() {
        if Some(index) == newest_user && !input.recall.trim().is_empty() {
            messages.push(ChatMessage {
                role: Role::System,
                blocks: vec![ContentBlock::Text(format!(
                    "<relevant_memory>\n{}\n</relevant_memory>",
                    input.recall
                ))],
            });
        }
        messages.push(message.clone());
    }
    messages.push(ChatMessage {
        role: Role::System,
        blocks: vec![ContentBlock::Text(input.runtime_note.clone())],
    });

    let transcript: Vec<Value> = history_messages
        .iter()
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
        tokens_for(&tools_text),
        tokens_for(&input.standing),
        tokens_for(&input.skills_index),
        tokens_for(&input.guidance),
        tokens_for(&transcript_text),
        tokens_for(&input.recall),
        tokens_for(&input.runtime_note),
    );
    let skills_text = input.skills_index.clone();
    let mut prefix_parts = BTreeMap::new();
    prefix_parts.insert("identity".to_owned(), hash_part(&input.soul));
    prefix_parts.insert("tools".to_owned(), hash_part(&tools_text));
    prefix_parts.insert("standing".to_owned(), hash_part(&input.standing));
    prefix_parts.insert("skills".to_owned(), hash_part(&skills_text));
    prefix_parts.insert("guidance".to_owned(), hash_part(&input.guidance));
    let prefix_hash = hash_part(
        &prefix_parts
            .iter()
            .map(|(name, hash)| format!("{name}:{hash}"))
            .collect::<Vec<_>>()
            .join("|"),
    );
    let document = json!({
        "identity": input.soul,
        "tools": tools,
        "standing_memory": input.standing,
        "standing_version": input.standing_version,
        "skills_index": input.skills_index,
        "guidance": input.guidance,
        "recall": input.recall,
        "transcript": transcript,
        "runtime": input.runtime_note,
        "model": input.model,
        "session_id": input.conversation_id,
        "prefix_hash": prefix_hash,
        "prefix_parts": prefix_parts,
        "tokens": {
            "identity": tokens.identity,
            "tools": tokens.tools,
            "standing": tokens.standing,
            "skills": tokens.skills,
            "guidance": tokens.guidance,
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
pub fn prefix_cause(previous: Option<&Value>, current: &BTreeMap<String, String>) -> Vec<String> {
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
            "tools" => "tools",
            "standing" => "standing memory",
            "skills" => "skills index",
            "guidance" => "AGENTS.md/HEARTH.md",
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

/// Build the history messages, then repair the tool-call pairing on the wire.
///
/// A steering message can be saved between an assistant call and its result.
/// OpenAI-compatible providers require every tool call to be answered directly
/// after the assistant message, so matching results are pulled forward and
/// unmatched calls or results are dropped.
fn paired_messages(history: &[Entry]) -> Vec<ChatMessage> {
    let messages: Vec<ChatMessage> = history.iter().filter_map(message_for_entry).collect();
    let mut output = Vec::with_capacity(messages.len());
    let mut queue: VecDeque<ChatMessage> = messages.into();
    while let Some(message) = queue.pop_front() {
        if message.role == Role::Tool {
            continue;
        }
        if message.role != Role::Assistant {
            output.push(message);
            continue;
        }
        let call_ids: HashSet<String> = message
            .blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall(call) => Some(call.id.clone()),
                _ => None,
            })
            .collect();
        if call_ids.is_empty() {
            output.push(message);
            continue;
        }

        let mut rest = VecDeque::with_capacity(queue.len());
        let mut matched = Vec::new();
        let mut satisfied = HashSet::new();
        while let Some(next) = queue.pop_front() {
            if next.role != Role::Tool {
                rest.push_back(next);
                continue;
            }
            let (mine, other) = split_tool_results(next.blocks, &call_ids, &satisfied);
            if !mine.is_empty() {
                for block in &mine {
                    if let ContentBlock::ToolResult { call_id, .. } = block {
                        satisfied.insert(call_id.clone());
                    }
                }
                matched.push(ChatMessage {
                    role: Role::Tool,
                    blocks: mine,
                });
            }
            if !other.is_empty() {
                rest.push_back(ChatMessage {
                    role: Role::Tool,
                    blocks: other,
                });
            }
        }

        let mut blocks = message.blocks;
        blocks.retain(|block| match block {
            ContentBlock::ToolCall(call) => satisfied.contains(&call.id),
            _ => true,
        });
        if !blocks.is_empty() {
            output.push(ChatMessage {
                role: Role::Assistant,
                blocks,
            });
        }
        output.extend(matched);
        queue = rest;
    }
    output
}

fn split_tool_results(
    blocks: Vec<ContentBlock>,
    call_ids: &HashSet<String>,
    satisfied: &HashSet<String>,
) -> (Vec<ContentBlock>, Vec<ContentBlock>) {
    let mut mine = Vec::new();
    let mut other = Vec::new();
    for block in blocks {
        match &block {
            ContentBlock::ToolResult { call_id, .. }
                if call_ids.contains(call_id) && !satisfied.contains(call_id) =>
            {
                mine.push(block);
            }
            _ => other.push(block),
        }
    }
    (mine, other)
}

/// Map one permanent entry to a chat message.
pub fn message_for_entry(entry: &Entry) -> Option<ChatMessage> {
    if entry.kind == "summary" {
        let text = crate::compaction::summary_text(entry)?;
        return Some(ChatMessage {
            role: Role::System,
            blocks: vec![ContentBlock::Text(format!(
                "<summary_of_earlier_conversation>\n{text}\n</summary_of_earlier_conversation>"
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
