//! The OpenAI-compatible chat completions adapter.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::provider::{Provider, ProviderError, ProviderStream, Sleeper, SystemSleeper};
use crate::types::{ChatRequest, ContentBlock, Role, StreamEvent};

/// A synchronous OpenAI-compatible provider.
pub struct OpenAiCompatibleProvider {
    base_url: String,
    api_key: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    session_header: Option<HeaderName>,
    client: reqwest::blocking::Client,
    #[allow(dead_code)]
    sleeper: Arc<dyn Sleeper>,
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
        let mut builder = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body);
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        if let (Some(header), Some(session)) = (&self.session_header, &request.session_id) {
            let value = HeaderValue::from_str(session).map_err(|error| {
                ProviderError::Config(format!("invalid session header value: {error}"))
            })?;
            builder = builder.header(header, value);
        }
        let response = builder
            .send()
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().unwrap_or_default();
            return Err(ProviderError::Request {
                status: status.as_u16(),
                message,
                retryable: false,
            });
        }
        Ok(Box::new(OpenAiStream::new(response)))
    }
}

struct OpenAiStream {
    reader: BufReader<reqwest::blocking::Response>,
    pending: VecDeque<Result<StreamEvent, ProviderError>>,
    done: bool,
    finish_reason: Option<String>,
}

impl OpenAiStream {
    fn new(response: reqwest::blocking::Response) -> Self {
        Self {
            reader: BufReader::new(response),
            pending: VecDeque::new(),
            done: false,
            finish_reason: None,
        }
    }

    fn handle_data(&mut self, data: &str) {
        if data == "[DONE]" {
            self.done = true;
            self.pending.push_back(Ok(StreamEvent::Done {
                finish_reason: self.finish_reason.take(),
            }));
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let Some(choices) = value["choices"].as_array() else {
            return;
        };
        for choice in choices {
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish_reason = Some(reason.to_owned());
            }
            if let Some(content) = choice["delta"]["content"].as_str() {
                self.pending
                    .push_back(Ok(StreamEvent::TextDelta(content.to_owned())));
            }
        }
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
                    self.pending.push_back(Ok(StreamEvent::Done {
                        finish_reason: self.finish_reason.take(),
                    }));
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
