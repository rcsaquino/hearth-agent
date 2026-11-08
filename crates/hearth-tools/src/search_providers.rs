//! Concrete search providers.

use std::time::Duration;

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};

use crate::search::{SearchError, SearchProvider, SearchResult};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const SNIPPET_CHARS: usize = 800;

/// Exa: keyless MCP by default, REST when an API key is present.
pub struct ExaSearch {
    mcp_base: String,
    api_base: String,
    api_key: Option<String>,
    client: Client,
}

impl ExaSearch {
    /// Build an Exa provider.
    pub fn new(
        mcp_base: impl Into<String>,
        api_base: impl Into<String>,
        api_key: Option<String>,
    ) -> Result<Self, SearchError> {
        Ok(Self {
            mcp_base: mcp_base.into(),
            api_base: api_base.into(),
            api_key,
            client: client()?,
        })
    }

    fn search_mcp(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        let url = format!(
            "{}/mcp?tools=web_search_advanced_exa",
            self.mcp_base.trim_end_matches('/')
        );
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "web_search_advanced_exa",
                "arguments": {
                    "query": query,
                    "type": "auto",
                    "numResults": limit,
                }
            }
        });
        let response = self
            .client
            .post(url)
            .header("Accept", "application/json, text/event-stream")
            .header("x-exa-source", "hearth")
            .json(&body)
            .send()
            .map_err(|error| SearchError::temporary(format!("Exa transport error: {error}")))?;
        let text = response_text(response, "Exa")?;
        let envelope = parse_rpc(&text)
            .ok_or_else(|| SearchError::temporary("Exa returned no JSON-RPC response"))?;
        if let Some(error) = envelope.get("error") {
            return Err(SearchError::permanent(format!(
                "Exa error: {}",
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            )));
        }
        let result = envelope
            .get("result")
            .ok_or_else(|| SearchError::permanent("Exa returned no result"))?;
        if result
            .get("_meta")
            .and_then(|meta| meta.get("ai.exa/rateLimited"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(SearchError::temporary(
                "Exa's keyless search hit its rate limit; add an Exa API key or use a fallback provider",
            ));
        }
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let message = result
                .get("content")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("Exa returned an error");
            return Err(SearchError::permanent(message.to_owned()));
        }
        let inner = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| SearchError::permanent("Exa returned no result text"))?;
        let parsed: Value = serde_json::from_str(inner)
            .map_err(|error| SearchError::permanent(format!("Exa returned invalid result JSON: {error}")))?;
        Ok(map_results(&parsed))
    }

    fn search_rest(&self, query: &str, limit: usize, api_key: &str) -> Result<Vec<SearchResult>, SearchError> {
        let url = format!("{}/search", self.api_base.trim_end_matches('/'));
        let body = json!({
            "query": query,
            "type": "auto",
            "numResults": limit,
        });
        let response = self
            .client
            .post(url)
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .header("x-exa-integration", "hearth")
            .json(&body)
            .send()
            .map_err(|error| SearchError::temporary(format!("Exa transport error: {error}")))?;
        let text = response_text(response, "Exa")?;
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::permanent(format!("Exa returned invalid JSON: {error}")))?;
        Ok(map_results(&parsed))
    }
}

impl SearchProvider for ExaSearch {
    fn name(&self) -> &'static str {
        "exa"
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        match &self.api_key {
            Some(key) if !key.is_empty() => self.search_rest(query, limit, key),
            _ => self.search_mcp(query, limit),
        }
    }
}

fn map_results(value: &Value) -> Vec<SearchResult> {
    let Some(results) = value.get("results").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut mapped = Vec::new();
    for item in results {
        let Some(url) = item.get("url").and_then(Value::as_str) else {
            continue;
        };
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(url)
            .to_owned();
        let snippet = item
            .get("highlights")
            .and_then(Value::as_array)
            .map(|highlights| {
                highlights
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|text| !text.trim().is_empty())
            .or_else(|| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let date = item
            .get("publishedDate")
            .and_then(Value::as_str)
            .map(str::to_owned);
        mapped.push(SearchResult {
            title,
            url: url.to_owned(),
            snippet: truncate(&snippet, SNIPPET_CHARS),
            date,
        });
    }
    mapped
}

fn parse_rpc(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text)
        && (value.get("result").is_some() || value.get("error").is_some())
    {
        return Some(value);
    }
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(data)
            && (value.get("result").is_some() || value.get("error").is_some())
        {
            return Some(value);
        }
    }
    None
}

fn response_text(response: Response, label: &str) -> Result<String, SearchError> {
    let status = response.status();
    let text = response
        .text()
        .unwrap_or_else(|error| format!("<body unreadable: {error}>"));
    if !status.is_success() {
        let retryable = status.as_u16() == 429 || status.is_server_error();
        return Err(SearchError {
            message: format!(
                "{label} error {}: {}",
                status.as_u16(),
                truncate(&text, 300)
            ),
            retryable,
        });
    }
    Ok(text)
}

fn client() -> Result<Client, SearchError> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| SearchError::permanent(format!("could not build the search client: {error}")))
}

fn truncate(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    let mut characters = trimmed.chars();
    let shortened: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

/// Brave Search: GET with an X-Subscription-Token.
pub struct BraveSearch {
    base: String,
    api_key: String,
    client: Client,
}

impl BraveSearch {
    /// Build a Brave provider.
    pub fn new(base: impl Into<String>, api_key: impl Into<String>) -> Result<Self, SearchError> {
        Ok(Self {
            base: base.into(),
            api_key: api_key.into(),
            client: client()?,
        })
    }
}

impl SearchProvider for BraveSearch {
    fn name(&self) -> &'static str {
        "brave"
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/web/search",
            self.base.trim_end_matches('/')
        ))
        .map_err(|error| SearchError::permanent(format!("invalid Brave base URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("count", &limit.to_string());
        let response = self
            .client
            .get(url)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .map_err(|error| SearchError::temporary(format!("Brave transport error: {error}")))?;
        let text = response_text(response, "Brave")?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::permanent(format!("Brave returned invalid JSON: {error}")))?;
        let mut results = Vec::new();
        if let Some(items) = value
            .get("web")
            .and_then(|web| web.get("results"))
            .and_then(Value::as_array)
        {
            for item in items {
                let Some(url) = item.get("url").and_then(Value::as_str) else {
                    continue;
                };
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .map(strip_html)
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| url.to_owned());
                let snippet = item
                    .get("description")
                    .and_then(Value::as_str)
                    .map(strip_html)
                    .unwrap_or_default();
                let date = item
                    .get("age")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("page_age").and_then(Value::as_str))
                    .map(str::to_owned);
                results.push(SearchResult {
                    title,
                    url: url.to_owned(),
                    snippet: truncate(&snippet, SNIPPET_CHARS),
                    date,
                });
            }
        }
        Ok(results)
    }
}

fn strip_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut inside = false;
    for character in value.chars() {
        match character {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(character),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_owned()
}

/// DeepSeek's tools search endpoint.
pub struct DeepSeekSearch {
    base: String,
    api_key: String,
    client: Client,
}

impl DeepSeekSearch {
    /// Build a DeepSeek provider.
    pub fn new(base: impl Into<String>, api_key: impl Into<String>) -> Result<Self, SearchError> {
        Ok(Self {
            base: base.into(),
            api_key: api_key.into(),
            client: client()?,
        })
    }
}

impl SearchProvider for DeepSeekSearch {
    fn name(&self) -> &'static str {
        "deepseek"
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        let url = format!("{}/v1/tools/search", self.base.trim_end_matches('/'));
        let body = json!({"query": query, "count": limit});
        let response = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .map_err(|error| SearchError::temporary(format!("DeepSeek transport error: {error}")))?;
        let text = response_text(response, "DeepSeek")?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::permanent(format!("DeepSeek returned invalid JSON: {error}")))?;
        let mut results = Vec::new();
        if let Some(items) = value.get("results").and_then(Value::as_array) {
            for item in items {
                let Some(url) = item.get("url").and_then(Value::as_str) else {
                    continue;
                };
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or(url)
                    .to_owned();
                let snippet = item
                    .get("summary")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("snippet").and_then(Value::as_str))
                    .unwrap_or_default();
                results.push(SearchResult {
                    title,
                    url: url.to_owned(),
                    snippet: truncate(snippet, SNIPPET_CHARS),
                    date: None,
                });
            }
        }
        Ok(results)
    }
}

/// Firecrawl's v2 search endpoint.
pub struct FirecrawlSearch {
    base: String,
    api_key: String,
    client: Client,
}

impl FirecrawlSearch {
    /// Build a Firecrawl provider.
    pub fn new(base: impl Into<String>, api_key: impl Into<String>) -> Result<Self, SearchError> {
        Ok(Self {
            base: base.into(),
            api_key: api_key.into(),
            client: client()?,
        })
    }
}

impl SearchProvider for FirecrawlSearch {
    fn name(&self) -> &'static str {
        "firecrawl"
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        let url = format!("{}/v2/search", self.base.trim_end_matches('/'));
        let body = json!({"query": query, "limit": limit, "sources": ["web"]});
        let response = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .map_err(|error| SearchError::temporary(format!("Firecrawl transport error: {error}")))?;
        let text = response_text(response, "Firecrawl")?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::permanent(format!("Firecrawl returned invalid JSON: {error}")))?;
        if value.get("success").and_then(Value::as_bool) == Some(false) {
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(SearchError::permanent(format!("Firecrawl error: {message}")));
        }
        let data = value.get("data");
        let items = data
            .and_then(|data| data.get("web"))
            .and_then(Value::as_array)
            .or_else(|| data.and_then(Value::as_array));
        let mut results = Vec::new();
        if let Some(items) = items {
            for item in items {
                let url = item
                    .get("url")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("metadata").and_then(|m| m.get("sourceURL")).and_then(Value::as_str))
                    .or_else(|| item.get("metadata").and_then(|m| m.get("url")).and_then(Value::as_str));
                let Some(url) = url else {
                    continue;
                };
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("metadata").and_then(|m| m.get("title")).and_then(Value::as_str))
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or(url)
                    .to_owned();
                let snippet = item
                    .get("description")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("snippet").and_then(Value::as_str))
                    .or_else(|| item.get("metadata").and_then(|m| m.get("description")).and_then(Value::as_str))
                    .unwrap_or_default();
                results.push(SearchResult {
                    title,
                    url: url.to_owned(),
                    snippet: truncate(snippet, SNIPPET_CHARS),
                    date: None,
                });
            }
        }
        Ok(results)
    }
}

/// TinyFish's search endpoint.
pub struct TinyFishSearch {
    base: String,
    api_key: String,
    client: Client,
}

impl TinyFishSearch {
    /// Build a TinyFish provider.
    pub fn new(base: impl Into<String>, api_key: impl Into<String>) -> Result<Self, SearchError> {
        Ok(Self {
            base: base.into(),
            api_key: api_key.into(),
            client: client()?,
        })
    }
}

impl SearchProvider for TinyFishSearch {
    fn name(&self) -> &'static str {
        "tinyfish"
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|error| SearchError::permanent(format!("invalid TinyFish base URL: {error}")))?;
        url.query_pairs_mut().append_pair("query", query);
        if limit > 10 {
            url.query_pairs_mut().append_pair("page", "0");
        }
        let response = self
            .client
            .get(url)
            .header("X-API-Key", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .map_err(|error| SearchError::temporary(format!("TinyFish transport error: {error}")))?;
        let text = response_text(response, "TinyFish")?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::permanent(format!("TinyFish returned invalid JSON: {error}")))?;
        let mut results = Vec::new();
        if let Some(items) = value.get("results").and_then(Value::as_array) {
            for item in items {
                let Some(url) = item
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.trim().is_empty())
                else {
                    continue;
                };
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or(url)
                    .to_owned();
                let snippet = item
                    .get("snippet")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let date = item.get("date").and_then(Value::as_str).map(str::to_owned);
                results.push(SearchResult {
                    title,
                    url: url.to_owned(),
                    snippet: truncate(snippet, SNIPPET_CHARS),
                    date,
                });
            }
        }
        Ok(results)
    }
}
