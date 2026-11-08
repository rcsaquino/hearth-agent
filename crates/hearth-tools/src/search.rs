//! Web search: an ordered provider chain and one model-facing tool.

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use hearth_model::ToolSpec;
use serde_json::{Value, json};
use thiserror::Error;

use crate::tool::{Tool, ToolError, ToolOutput};

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub date: Option<String>,
}

/// A search provider failure.
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct SearchError {
    pub message: String,
    pub retryable: bool,
}

impl SearchError {
    /// A caller or response error that a retry will not fix.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    /// A temporary failure.
    pub fn temporary(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}

/// One place results can come from.
pub trait SearchProvider: Send + Sync {
    /// A short stable name.
    fn name(&self) -> &'static str;

    /// Run one query and return up to `limit` results.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError>;
}

/// The model-facing web_search tool.
pub struct WebSearchTool {
    providers: Vec<Arc<dyn SearchProvider>>,
    parallel: usize,
    results: usize,
    max_results: usize,
}

impl WebSearchTool {
    /// Build a tool over an ordered provider chain.
    pub fn new(
        providers: Vec<Arc<dyn SearchProvider>>,
        parallel: usize,
        results: usize,
        max_results: usize,
    ) -> Self {
        Self {
            providers,
            parallel: parallel.clamp(1, 3),
            results: results.clamp(1, 20),
            max_results: max_results.clamp(1, 50),
        }
    }
}

impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".to_owned(),
            description: "Search the web with one or more queries. Returns titles, URLs, snippets, and dates. No answer synthesis and no scraping.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "queries": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1,
                        "maxItems": 10
                    }
                },
                "required": ["queries"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let queries = parse_queries(arguments)?;
        if self.providers.is_empty() {
            return Err(ToolError::invalid(
                "no_provider",
                "no web search provider is configured",
            ));
        }

        let mut outcomes: Vec<Option<Result<Vec<SearchResult>, SearchError>>> =
            (0..queries.len()).map(|_| None).collect();
        for (chunk_index, chunk) in queries.chunks(self.parallel).enumerate() {
            let start = chunk_index * self.parallel;
            thread::scope(|scope| {
                let mut handles = Vec::with_capacity(chunk.len());
                for (offset, query) in chunk.iter().enumerate() {
                    let providers = &self.providers;
                    let limit = self.results;
                    let query = query.as_str();
                    handles.push(scope.spawn(move || {
                        (start + offset, search_with(&providers[..], query, limit))
                    }));
                }
                for handle in handles {
                    match handle.join() {
                        Ok((index, outcome)) => outcomes[index] = Some(outcome),
                        Err(_) => {
                            // A panicking provider must not take the whole tool down.
                            // Its slot stays None and is reported as a failure below.
                        }
                    }
                }
            });
        }

        let mut search_results = Vec::new();
        let mut errors = Vec::new();
        let mut seen = HashSet::new();
        let mut all_failed = true;
        let mut all_retryable = true;
        let mut first_error: Option<SearchError> = None;

        for (query, outcome) in queries.iter().zip(outcomes) {
            match outcome {
                Some(Ok(items)) => {
                    all_failed = false;
                    for item in items {
                        if seen.insert(item.url.clone()) {
                            search_results.push(item);
                        }
                    }
                }
                Some(Err(error)) => {
                    if first_error.is_none() {
                        first_error = Some(error.clone());
                    }
                    all_retryable &= error.retryable;
                    errors.push(json!({"query": query, "error": error.message}));
                }
                None => {
                    all_retryable = false;
                    errors.push(json!({"query": query, "error": "search provider panicked"}));
                }
            }
        }

        if all_failed {
            let error = first_error
                .unwrap_or_else(|| SearchError::permanent("every search provider failed"));
            return Err(ToolError {
                code: "search_failed".to_owned(),
                message: error.message,
                retryable: all_retryable && error.retryable,
                detail: None,
            });
        }

        search_results.truncate(self.max_results);
        let results: Vec<Value> = search_results
            .iter()
            .map(|result| {
                json!({
                    "title": result.title,
                    "url": result.url,
                    "snippet": result.snippet,
                    "date": result.date,
                })
            })
            .collect();
        Ok(ToolOutput {
            content: json!({"results": results, "errors": errors}).to_string(),
            is_error: false,
        })
    }
}

fn parse_queries(arguments: &Value) -> Result<Vec<String>, ToolError> {
    let values = arguments
        .get("queries")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::invalid("invalid_arguments", "queries must be an array"))?;
    if values.is_empty() {
        return Err(ToolError::invalid(
            "invalid_arguments",
            "at least one query is required",
        ));
    }
    if values.len() > 10 {
        return Err(ToolError::invalid(
            "invalid_arguments",
            "at most ten queries are allowed in one call",
        ));
    }
    let mut queries = Vec::with_capacity(values.len());
    for value in values {
        let query = value
            .as_str()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| {
                ToolError::invalid(
                    "invalid_arguments",
                    "every query must be a non-empty string",
                )
            })?;
        queries.push(query.to_owned());
    }
    Ok(queries)
}

fn search_with(
    providers: &[Arc<dyn SearchProvider>],
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let mut first_error = None;
    for provider in providers {
        match provider.search(query, limit) {
            Ok(mut results) => {
                results.truncate(limit);
                return Ok(results);
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    Err(first_error.unwrap_or_else(|| SearchError::permanent("no search provider available")))
}
