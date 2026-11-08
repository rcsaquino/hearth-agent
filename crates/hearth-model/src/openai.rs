//! The OpenAI-compatible chat completions adapter.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::provider::{Provider, ProviderError, ProviderStream, Sleeper, SystemSleeper};
use crate::types::{ChatRequest, ContentBlock, Role, StreamEvent, ToolCall, Usage};

/// A synchronous OpenAI-compatible provider.
pub struct OpenAiCompatibleProvider {
    base_url: String,
    api_key: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    session_header: Option<HeaderName>,
    client: reqwest::blocking::Client,
    sleeper: Arc<dyn Sleeper>,
}

const MAX_ATTEMPTS: u32 = 3;
const BASE_DELAY: Duration = Duration::from_millis(500);

fn backoff(attempt: u32) -> Duration {
    BASE_DELAY.saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
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
            client,
            sleeper,
        })
    }

    fn body(&self, request: &ChatRequest) -> Result<Value, ProviderError> {
        let mut messages = Vec::with_capacity(request.messages.len());
        for message in &request.messages {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant | Role::Tool => {
                    return Err(ProviderError::Config(
                        "assistant and tool messages are not translated yet".to_owned(),
                    ));
                }
            };
            let text = message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();
            messages.push(json!({"role": role, "content": text}));
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
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
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
