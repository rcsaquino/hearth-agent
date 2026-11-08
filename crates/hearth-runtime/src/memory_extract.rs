//! Post-run memory extraction from a settled conversation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use hearth_core::{
    CoreError, Entry, NewTask, NewUsage, Session, StepOutput, TerminalOutcome, parse_blocks,
};
use hearth_memory::{
    MemoryKind, MemoryPatch, MemoryService, NewMemoryInput, SourceKind, is_trivial,
};
use hearth_model::{
    ChatMessage, ChatRequest, ContentBlock, Pricing, Provider, Role, StreamEvent, Usage,
};
use serde_json::{Value, json};
use thiserror::Error;

const MAX_OPERATIONS: usize = 3;
const VIEW_TOKEN_CAP: usize = 2000;
const EXTRACTION_MAX_TOKENS: u32 = 1024;

const INSTRUCTION: &str = "You extract durable memory from a completed run. \
Return valid JSON only: {\"operations\":[...]}. \
At most three operations. Each operation is {\"op\":\"add\"|\"update\"|\"supersede\"|\"noop\", \
\"kind\":\"fact\"|\"preference\"|\"person\"|\"project\"|\"event\"|\"episode\"|\"rule\", \
\"content\":\"...\",\"tags\":[\"...\"],\"importance\":0-100, \
\"evidence_entry_id\":\"...\",\"evidence_quote\":\"...\",\"supersedes_id\":\"...\"}. \
Use one durable fact per operation. The evidence quote must appear verbatim in the cited entry. \
Do not store secrets or scratch state. Use update or supersede with supersedes_id when an \
existing memory should change. Return {\"operations\":[{\"op\":\"noop\"}]} when nothing qualifies.";

/// A failure that stopped the extraction run.
#[derive(Debug, Error)]
pub enum MemoryExtractError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("extraction response was invalid: {0}")]
    Parse(String),
    #[error("{0}")]
    Invalid(String),
}

/// Runs one bounded extraction request after an answer.
pub struct MemoryExtractor {
    session: Session,
    provider: Arc<dyn Provider>,
    pricing: Pricing,
    memory: Arc<MemoryService>,
    model: String,
    provider_name: String,
    owner: String,
}

impl MemoryExtractor {
    /// Build an extractor.
    pub fn new(
        session: Session,
        provider: Arc<dyn Provider>,
        pricing: Pricing,
        memory: Arc<MemoryService>,
        model: String,
        provider_name: String,
        owner: String,
    ) -> Self {
        Self {
            session,
            provider,
            pricing,
            memory,
            model,
            provider_name,
            owner,
        }
    }

    /// Extract and apply memory from the run entries.
    pub fn extract(
        &self,
        conversation_id: &str,
        submission_id: &str,
        entries: &[Entry],
    ) -> Result<(), MemoryExtractError> {
        let query = entries
            .iter()
            .rev()
            .find(|entry| entry.kind == "user")
            .map(entry_text)
            .unwrap_or_default();
        if is_trivial(&query) {
            return Ok(());
        }
        let (view, entry_texts) = run_view(entries);
        let similar = self
            .memory
            .search(&query, 5)
            .map(|items| {
                items
                    .iter()
                    .map(|item| self.memory.render_line(item))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let similar = if similar.is_empty() {
            "(none)".to_owned()
        } else {
            similar
        };
        let user_text = format!("## Run\n{view}\n\n## Similar memories\n{similar}");
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    blocks: vec![ContentBlock::Text(INSTRUCTION.to_owned())],
                },
                ChatMessage {
                    role: Role::User,
                    blocks: vec![ContentBlock::Text(user_text.clone())],
                },
            ],
            tools: Vec::new(),
            temperature: None,
            max_tokens: Some(EXTRACTION_MAX_TOKENS),
            session_id: Some(conversation_id.to_owned()),
        };
        let prompt_text = format!("{INSTRUCTION}\n\n{user_text}");
        self.session.save_prompt_blob(&prompt_text)?;

        let entry_ids: Vec<String> = entries.iter().map(|entry| entry.id.clone()).collect();
        let task = self.session.create_task(
            NewTask::new(
                "memory.extract",
                1,
                "extract",
                json!({
                    "submission_id": submission_id,
                    "conversation_id": conversation_id,
                    "entry_ids": entry_ids,
                }),
            )
            .conversation(conversation_id),
        )?;
        let claimed = self
            .session
            .claim_task_by_id(&task.id, &self.owner, 60_000_000)?
            .ok_or_else(|| {
                MemoryExtractError::Invalid("the extraction task could not be claimed".to_owned())
            })?;

        let started = Instant::now();
        let stream = match self.provider.stream(request) {
            Ok(stream) => stream,
            Err(error) => {
                self.fail_task(&claimed.id, &format!("provider error: {error}"))?;
                return Err(MemoryExtractError::Provider(error.to_string()));
            }
        };
        let mut text = String::new();
        let mut usage = Usage::default();
        for event in stream {
            match event {
                Ok(StreamEvent::TextDelta(delta)) => text.push_str(&delta),
                Ok(StreamEvent::Usage(value)) => usage = value,
                Ok(_) => {}
                Err(error) => {
                    let latency_ms = started.elapsed().as_millis() as i64;
                    if has_usage(usage) {
                        let _ = self.record_usage(&claimed.id, conversation_id, usage, latency_ms);
                    }
                    self.fail_task(&claimed.id, &format!("provider error: {error}"))?;
                    return Err(MemoryExtractError::Provider(error.to_string()));
                }
            }
        }
        let latency_ms = started.elapsed().as_millis() as i64;
        self.record_usage(&claimed.id, conversation_id, usage, latency_ms)?;

        let operations = match parse_operations(&text) {
            Ok(operations) => operations,
            Err(error) => {
                self.fail_task(&claimed.id, &error.to_string())?;
                return Err(error);
            }
        };
        let id_set: HashSet<String> = entry_ids.iter().cloned().collect();
        let (applied, rejected) = self.apply_operations(&operations, &entry_texts, &id_set);
        self.session.finish_task(
            &claimed.id,
            &self.owner,
            StepOutput::Terminal {
                outcome: TerminalOutcome::Done,
                result: json!({"applied": applied, "rejected": rejected}),
            },
        )?;
        Ok(())
    }

    fn apply_operations(
        &self,
        operations: &[Value],
        entry_texts: &HashMap<String, String>,
        entry_ids: &HashSet<String>,
    ) -> (usize, usize) {
        let mut applied = 0usize;
        let mut rejected = 0usize;
        for operation in operations.iter().take(MAX_OPERATIONS) {
            match self.apply_one(operation, entry_texts, entry_ids) {
                Ok(true) => applied += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!(%error, "memory extraction operation rejected");
                    rejected += 1;
                }
            }
        }
        (applied, rejected)
    }

    fn apply_one(
        &self,
        operation: &Value,
        entry_texts: &HashMap<String, String>,
        entry_ids: &HashSet<String>,
    ) -> Result<bool, String> {
        let op = operation
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| "operation has no op".to_owned())?;
        if op == "noop" {
            return Ok(false);
        }
        if !matches!(op, "add" | "update" | "supersede") {
            return Err(format!("unknown operation: {op}"));
        }
        let entry_id = operation
            .get("evidence_entry_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "operation has no evidence_entry_id".to_owned())?;
        if !entry_ids.contains(entry_id) {
            return Err(format!("evidence entry is not part of the run: {entry_id}"));
        }
        let quote = operation
            .get("evidence_quote")
            .and_then(Value::as_str)
            .ok_or_else(|| "operation has no evidence_quote".to_owned())?;
        let entry = entry_texts
            .get(entry_id)
            .ok_or_else(|| "evidence entry has no text".to_owned())?;
        let quote = normalize_ws(quote);
        if quote.is_empty() || !normalize_ws(entry).contains(&quote) {
            return Err("evidence quote does not appear in the entry".to_owned());
        }

        let kind_name = operation
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("fact");
        let kind = MemoryKind::parse(kind_name)
            .ok_or_else(|| format!("unknown memory kind: {kind_name}"))?;
        let content = operation
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|content| !content.is_empty())
            .ok_or_else(|| "operation has no content".to_owned())?;
        let tags: Vec<String> = operation
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let importance = operation
            .get("importance")
            .and_then(Value::as_i64)
            .unwrap_or(50);
        if !(0..=100).contains(&importance) {
            return Err("importance must be between 0 and 100".to_owned());
        }

        match op {
            "add" => {
                self.memory
                    .save_with_actor(
                        NewMemoryInput {
                            content: content.to_owned(),
                            kind,
                            tags,
                            importance,
                            source_kind: SourceKind::Session,
                            source_ref: Some(entry_id.to_owned()),
                            supersedes: None,
                            valid_from: None,
                            valid_to: None,
                        },
                        "system",
                    )
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            "update" => {
                let target = operation
                    .get("supersedes_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "update needs supersedes_id".to_owned())?;
                self.memory
                    .update(
                        target,
                        MemoryPatch {
                            content: Some(content.to_owned()),
                            tags: Some(tags),
                            importance: Some(importance),
                            ..MemoryPatch::default()
                        },
                    )
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            "supersede" => {
                let target = operation
                    .get("supersedes_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "supersede needs supersedes_id".to_owned())?;
                self.memory
                    .save_with_actor(
                        NewMemoryInput {
                            content: content.to_owned(),
                            kind,
                            tags,
                            importance,
                            source_kind: SourceKind::Session,
                            source_ref: Some(entry_id.to_owned()),
                            supersedes: Some(target.to_owned()),
                            valid_from: None,
                            valid_to: None,
                        },
                        "system",
                    )
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            _ => Err(format!("unknown operation: {op}")),
        }
    }

    fn record_usage(
        &self,
        task_id: &str,
        conversation_id: &str,
        usage: Usage,
        latency_ms: i64,
    ) -> Result<i64, MemoryExtractError> {
        let cost_micros = self
            .pricing
            .price(&self.model)
            .map(|price| price.cost_micros(&usage))
            .unwrap_or(0);
        self.session.record_usage(NewUsage {
            task_id: Some(task_id.to_owned()),
            conversation_id: Some(conversation_id.to_owned()),
            provider: self.provider_name.clone(),
            model: self.model.clone(),
            kind: "memory.extract".to_owned(),
            usage,
            cost_micros,
            latency_ms,
        })?;
        Ok(cost_micros)
    }

    fn fail_task(&self, task_id: &str, message: &str) -> Result<(), CoreError> {
        self.session.finish_task(
            task_id,
            &self.owner,
            StepOutput::Terminal {
                outcome: TerminalOutcome::Failed,
                result: json!({"error": short(message)}),
            },
        )?;
        Ok(())
    }
}

fn run_view(entries: &[Entry]) -> (String, HashMap<String, String>) {
    let mut view = String::new();
    let mut texts = HashMap::new();
    for entry in entries {
        let text = entry_text(entry);
        texts.insert(entry.id.clone(), text.clone());
        if text.trim().is_empty() {
            continue;
        }
        let line = format!("[{}] {}: {}\n", entry.id, entry.kind, text);
        if hearth_model::estimate_tokens(&format!("{view}{line}")) > VIEW_TOKEN_CAP as i64 {
            continue;
        }
        view.push_str(&line);
    }
    (view, texts)
}

fn entry_text(entry: &Entry) -> String {
    parse_blocks(&entry.content)
        .into_iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text),
            ContentBlock::Thinking(_) => None,
            ContentBlock::ToolCall(call) => {
                Some(format!("tool call {} {}", call.name, call.arguments))
            }
            ContentBlock::ToolResult { content, .. } => Some(format!("tool result: {content}")),
            ContentBlock::Image { path, .. } => Some(format!("[image {path}]")),
            ContentBlock::File { name, .. } => Some(format!("[file {name}]")),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_operations(text: &str) -> Result<Vec<Value>, MemoryExtractError> {
    let start = text
        .find('{')
        .ok_or_else(|| MemoryExtractError::Parse("no JSON object was found".to_owned()))?;
    let end = text
        .rfind('}')
        .ok_or_else(|| MemoryExtractError::Parse("no JSON object was found".to_owned()))?;
    let value: Value = serde_json::from_str(&text[start..=end])
        .map_err(|error| MemoryExtractError::Parse(error.to_string()))?;
    let operations = value
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| MemoryExtractError::Parse("operations is missing".to_owned()))?;
    Ok(operations.clone())
}

fn normalize_ws(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn has_usage(usage: Usage) -> bool {
    usage.input_tokens > 0
        || usage.cache_read_tokens > 0
        || usage.cache_write_tokens > 0
        || usage.output_tokens > 0
        || usage.reasoning_tokens > 0
}

fn short(message: &str) -> String {
    let single = message.replace(['\n', '\r'], " ");
    let text: String = single.trim().chars().take(200).collect();
    if text.is_empty() {
        "memory extraction failed".to_owned()
    } else {
        text
    }
}
