//! Provider-neutral chat types and the OpenAI-compatible adapter.

mod openai;
mod provider;
mod types;

pub use openai::OpenAiCompatibleProvider;
pub use provider::{Provider, ProviderError, ProviderStream, Sleeper, SystemSleeper};
pub use types::{
    ChatMessage, ChatRequest, ContentBlock, Role, StreamEvent, ToolCall, ToolSpec, Usage,
};
