//! Provider-neutral chat types and the OpenAI-compatible adapter.

mod estimate;
mod media;
mod openai;
mod price;
mod provider;
mod types;

pub use estimate::estimate_tokens;
pub use media::{MediaError, Speaker, SpeechClient, Transcriber, TranscriptionClient};
pub use openai::OpenAiCompatibleProvider;
pub use price::{Price, Pricing};
pub use provider::{Provider, ProviderError, ProviderStream, Sleeper, SystemSleeper};
pub use types::{
    ChatMessage, ChatRequest, ContentBlock, Role, StreamEvent, ToolCall, ToolSpec, Usage,
};
