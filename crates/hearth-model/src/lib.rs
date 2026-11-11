//! Provider-neutral chat types and the OpenAI-compatible adapter.

mod catalog;
mod estimate;
mod media;
mod openai;
mod provider;
mod types;

pub use catalog::{
    Catalog, CatalogError, ModelLimits, fetch as fetch_catalog, lookup as lookup_model_info,
};
pub use estimate::estimate_tokens;
pub use media::{
    ELEVENLABS_TAG_GUIDANCE, ImageClient, MediaError, Speaker, SpeechClient, Transcriber,
    TranscriptionClient,
};
pub use openai::{OpenAiCompatibleProvider, list_models};
pub use provider::{Provider, ProviderError, ProviderStream, Sleeper, SystemSleeper};
pub use types::{
    ChatMessage, ChatRequest, ContentBlock, Role, StreamEvent, ToolCall, ToolSpec, Usage,
};
