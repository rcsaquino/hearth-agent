//! Provider-neutral request and stream types.

use serde_json::Value;

/// Who produced a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One provider-neutral piece of a message.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    Text(String),
    Thinking(String),
    ToolCall(ToolCall),
    ToolResult {
        call_id: String,
        content: String,
    },
    Image {
        path: String,
        mime: String,
    },
    File {
        path: String,
        name: String,
        mime: String,
    },
}

/// One message in a chat request.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
}

/// A tool call proposed by the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Token counts reported by a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
}

/// One chat request.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub session_id: Option<String>,
}

/// One event from a streaming response.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCall(ToolCall),
    Usage(Usage),
    Done { finish_reason: Option<String> },
}
