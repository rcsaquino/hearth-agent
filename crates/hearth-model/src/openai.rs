//! The OpenAI-compatible chat completions adapter.

use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::provider::{Provider, ProviderError, ProviderStream, Sleeper, SystemSleeper};
use crate::types::{ChatRequest, ContentBlock, Role, StreamEvent, ToolCall, Usage};

/// A synchronous OpenAI-compatible provider.
pub struct OpenAiCompatibleProvider {
    base_url: String,
    api_key: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    session_header: Option<HeaderName>,
    reasoning_effort: Option<String>,
    client: reqwest::blocking::Client,
    sleeper: Arc<dyn Sleeper>,
}

const MAX_ATTEMPTS: u32 = 3;
const BASE_DELAY: Duration = Duration::from_millis(500);
const MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(15);

fn backoff(attempt: u32) -> Duration {
    BASE_DELAY.saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
}

/// List model ids from an OpenAI-compatible `/models` endpoint.
pub fn list_models(base_url: &str, api_key: &str) -> Result<Vec<String>, ProviderError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(MODEL_LIST_TIMEOUT)
        .build()
        .map_err(|error| {
            ProviderError::Config(format!("could not build the HTTP client: {error}"))
        })?;
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let response = client
        .get(&url)
        .bearer_auth(api_key)
        .send()
        .map_err(|error| ProviderError::Transport(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        let message: String = body.chars().take(200).collect();
        return Err(ProviderError::Request {
            status: status.as_u16(),
            message,
            retryable: false,
        });
    }
    let body: Value = response
        .json()
        .map_err(|error| ProviderError::Protocol(format!("model list is not JSON: {error}")))?;
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array());
    let mut models = Vec::new();
    for item in items.into_iter().flatten() {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.as_str())
            .filter(|id| !id.is_empty());
        if let Some(id) = id {
            models.push(id.to_owned());
        }
    }
    models.sort();
    models.dedup();
    Ok(models)
}

impl OpenAiCompatibleProvider {
    /// Build a provider with the production sleeper.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        headers: Vec<(String, String)>,
        session_header: Option<String>,
    ) -> Result<Self, ProviderError> {
        Self::with_sleeper(
            base_url,
            api_key,
            headers,
            session_header,
            Arc::new(SystemSleeper),
        )
    }

    /// Build a provider with an injected sleeper.
    pub fn with_sleeper(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        headers: Vec<(String, String)>,
        session_header: Option<String>,
        sleeper: Arc<dyn Sleeper>,
    ) -> Result<Self, ProviderError> {
        let headers = headers
            .into_iter()
            .map(|(name, value)| {
                let name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                    ProviderError::Config(format!("invalid header name: {error}"))
                })?;
                let value = HeaderValue::from_str(&value).map_err(|error| {
                    ProviderError::Config(format!("invalid header value: {error}"))
                })?;
                Ok((name, value))
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        let session_header = match session_header {
            Some(name) => Some(HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                ProviderError::Config(format!("invalid session header name: {error}"))
            })?),
            None => None,
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| {
                ProviderError::Config(format!("could not build the HTTP client: {error}"))
            })?;
        Ok(Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            headers,
            session_header,
            reasoning_effort: None,
            client,
            sleeper,
        })
    }

    /// Send this reasoning effort on every request.
    ///
    /// `default` leaves the provider's own behavior unchanged.
    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort.filter(|effort| !effort.is_empty() && effort != "default");
        self
    }

    /// Build the exact wire body without sending it.
    pub fn wire_body(&self, request: &ChatRequest) -> Result<Value, ProviderError> {
        self.body(request)
    }

    fn body(&self, request: &ChatRequest) -> Result<Value, ProviderError> {
        let mut messages = Vec::with_capacity(request.messages.len());
        for message in &request.messages {
            match message.role {
                Role::System | Role::User => {
                    let role = if message.role == Role::System {
                        "system"
                    } else {
                        "user"
                    };
                    let has_image = message
                        .blocks
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Image { .. }));
                    if role == "user" && has_image {
                        let mut parts = Vec::new();
                        for block in &message.blocks {
                            match block {
                                ContentBlock::Text(text) => {
                                    parts.push(json!({"type": "text", "text": text}));
                                }
                                ContentBlock::Image { path, .. } => {
                                    parts.push(json!({
                                        "type": "image_url",
                                        "image_url": {"url": image_data_url(path)?},
                                    }));
                                }
                                ContentBlock::File { path, name, .. } => {
                                    parts.push(json!({
                                        "type": "text",
                                        "text": file_note(name, path),
                                    }));
                                }
                                _ => {}
                            }
                        }
                        messages.push(json!({"role": role, "content": parts}));
                    } else {
                        messages.push(json!({
                            "role": role,
                            "content": text_blocks(&message.blocks),
                        }));
                    }
                }
                Role::Assistant => {
                    let text = text_blocks(&message.blocks);
                    let mut tool_calls = Vec::new();
                    for block in &message.blocks {
                        if let ContentBlock::ToolCall(call) = block {
                            let arguments =
                                serde_json::to_string(&call.arguments).map_err(|error| {
                                    ProviderError::Config(format!(
                                        "tool call arguments could not be serialized: {error}"
                                    ))
                                })?;
                            tool_calls.push(json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": arguments,
                                }
                            }));
                        }
                    }
                    let mut value = json!({"role": "assistant"});
                    value["content"] = if text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(text)
                    };
                    if !tool_calls.is_empty() {
                        value["tool_calls"] = json!(tool_calls);
                    }
                    messages.push(value);
                }
                Role::Tool => {
                    for block in &message.blocks {
                        if let ContentBlock::ToolResult { call_id, content } = block {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": call_id,
                                "content": content,
                            }));
                        }
                    }
                }
            }
        }

        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if let Some(effort) = &self.reasoning_effort {
            if effort == "off" {
                // DeepSeek disables thinking with its own toggle, not an effort.
                if self.base_url.contains("deepseek.com") {
                    body["thinking"] = json!({"type": "disabled"});
                } else {
                    body["reasoning_effort"] = json!("none");
                }
            } else {
                body["reasoning_effort"] = json!(effort);
            }
        }
        Ok(body)
    }
}

impl Provider for OpenAiCompatibleProvider {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        let body = self.body(&request)?;
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let session_value = match (&self.session_header, &request.session_id) {
            (Some(_), Some(session)) => Some(HeaderValue::from_str(session).map_err(|error| {
                ProviderError::Config(format!("invalid session header value: {error}"))
            })?),
            _ => None,
        };
        for attempt in 1..=MAX_ATTEMPTS {
            let mut builder = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body);
            for (name, value) in &self.headers {
                builder = builder.header(name, value);
            }
            if let (Some(header), Some(value)) = (&self.session_header, &session_value) {
                builder = builder.header(header, value);
            }
            match builder.send() {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        let content_type = response
                            .headers()
                            .get(CONTENT_TYPE)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_owned();
                        if !content_type
                            .trim_start()
                            .to_ascii_lowercase()
                            .starts_with("text/event-stream")
                        {
                            let body = response.text().unwrap_or_default();
                            let snippet: String = body.chars().take(200).collect();
                            return Err(ProviderError::Protocol(format!(
                                "provider returned {content_type:?} instead of an event stream: {snippet}"
                            )));
                        }
                        return Ok(Box::new(OpenAiStream::new(response)));
                    }
                    let retryable = matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
                    let message = response.text().unwrap_or_default();
                    let error = ProviderError::Request {
                        status: status.as_u16(),
                        message,
                        retryable,
                    };
                    if retryable && attempt < MAX_ATTEMPTS {
                        self.sleeper.sleep(backoff(attempt));
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => {
                    if attempt < MAX_ATTEMPTS {
                        self.sleeper.sleep(backoff(attempt));
                        continue;
                    }
                    return Err(ProviderError::Transport(error.to_string()));
                }
            }
        }
        Err(ProviderError::Transport(
            "provider retry attempts exhausted".to_owned(),
        ))
    }
}

struct OpenAiStream {
    reader: BufReader<reqwest::blocking::Response>,
    pending: VecDeque<Result<StreamEvent, ProviderError>>,
    done: bool,
    finish_reason: Option<String>,
    tool_calls: BTreeMap<usize, PartialToolCall>,
    usage_sent: bool,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl PartialToolCall {
    fn into_tool_call(self) -> Result<ToolCall, ProviderError> {
        if self.id.is_empty() || self.name.is_empty() {
            return Err(ProviderError::Protocol(
                "tool call is missing its id or name".to_owned(),
            ));
        }
        let arguments = if self.arguments.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&self.arguments).map_err(|error| {
                ProviderError::Protocol(format!("tool call arguments are not JSON: {error}"))
            })?
        };
        Ok(ToolCall {
            id: self.id,
            name: self.name,
            arguments,
        })
    }
}

impl OpenAiStream {
    fn new(response: reqwest::blocking::Response) -> Self {
        Self {
            reader: BufReader::new(response),
            pending: VecDeque::new(),
            done: false,
            finish_reason: None,
            tool_calls: BTreeMap::new(),
            usage_sent: false,
        }
    }

    fn handle_data(&mut self, data: &str) {
        if data == "[DONE]" {
            self.done = true;
            let events = self.finish();
            self.pending.extend(events);
            return;
        }
        let value = match serde_json::from_str::<Value>(data) {
            Ok(value) => value,
            Err(error) => {
                self.done = true;
                self.pending.push_back(Err(ProviderError::Protocol(format!(
                    "invalid SSE JSON: {error}"
                ))));
                return;
            }
        };
        if !self.usage_sent && value.get("usage").is_some_and(|usage| !usage.is_null()) {
            self.pending
                .push_back(Ok(StreamEvent::Usage(parse_usage(&value["usage"]))));
            self.usage_sent = true;
        }
        let Some(choices) = value["choices"].as_array() else {
            return;
        };
        for choice in choices {
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish_reason = Some(reason.to_owned());
            }
            if let Some(reasoning) = choice["delta"]["reasoning_content"]
                .as_str()
                .or_else(|| choice["delta"]["reasoning"].as_str())
            {
                self.pending
                    .push_back(Ok(StreamEvent::ThinkingDelta(reasoning.to_owned())));
            }
            if let Some(content) = choice["delta"]["content"].as_str() {
                self.pending
                    .push_back(Ok(StreamEvent::TextDelta(content.to_owned())));
            }
            if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
                for call in calls {
                    let index = call["index"].as_u64().unwrap_or(0) as usize;
                    let partial = self.tool_calls.entry(index).or_default();
                    if let Some(id) = call["id"].as_str() {
                        partial.id = id.to_owned();
                    }
                    if let Some(name) = call["function"]["name"].as_str() {
                        partial.name.push_str(name);
                    }
                    if let Some(arguments) = call["function"]["arguments"].as_str() {
                        partial.arguments.push_str(arguments);
                    }
                }
            }
        }
    }

    fn finish(&mut self) -> Vec<Result<StreamEvent, ProviderError>> {
        let mut events = Vec::new();
        for (_, partial) in std::mem::take(&mut self.tool_calls) {
            match partial.into_tool_call() {
                Ok(call) => events.push(Ok(StreamEvent::ToolCall(call))),
                Err(error) => {
                    events.push(Err(error));
                    self.done = true;
                    return events;
                }
            }
        }
        events.push(Ok(StreamEvent::Done {
            finish_reason: self.finish_reason.take(),
        }));
        events
    }
}

impl Iterator for OpenAiStream {
    type Item = Result<StreamEvent, ProviderError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(item);
            }
            if self.done {
                return None;
            }
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    self.done = true;
                    let events = self.finish();
                    self.pending.extend(events);
                }
                Ok(_) => {
                    let line = line.trim_end_matches(['\r', '\n']);
                    if let Some(data) = line.strip_prefix("data:") {
                        self.handle_data(data.trim_start());
                    }
                }
                Err(error) => {
                    self.done = true;
                    return Some(Err(ProviderError::Transport(error.to_string())));
                }
            }
        }
    }
}

fn parse_usage(value: &Value) -> Usage {
    Usage {
        input_tokens: value["prompt_tokens"].as_i64().unwrap_or(0),
        cache_read_tokens: value["prompt_tokens_details"]["cached_tokens"]
            .as_i64()
            .or_else(|| value["cache_read_input_tokens"].as_i64())
            .unwrap_or(0),
        cache_write_tokens: value["cache_creation_input_tokens"].as_i64().unwrap_or(0),
        output_tokens: value["completion_tokens"].as_i64().unwrap_or(0),
        reasoning_tokens: value["completion_tokens_details"]["reasoning_tokens"]
            .as_i64()
            .unwrap_or(0),
    }
}

fn text_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.clone()),
            ContentBlock::File { path, name, .. } => Some(file_note(name, path)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn file_note(name: &str, path: &str) -> String {
    format!("[file: {name} ({path})]")
}

fn image_data_url(path: &str) -> Result<String, ProviderError> {
    let bytes = std::fs::read(path)
        .map_err(|error| ProviderError::Protocol(format!("image could not be read: {error}")))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|error| ProviderError::Protocol(format!("image could not be decoded: {error}")))?;
    let image = if image.width().max(image.height()) > 1568 {
        image.thumbnail(1568, 1568)
    } else {
        image
    };
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| ProviderError::Protocol(format!("image could not be encoded: {error}")))?;
    let base64 = base64::engine::general_purpose::STANDARD.encode(encoded.into_inner());
    Ok(format!("data:image/png;base64,{base64}"))
}
