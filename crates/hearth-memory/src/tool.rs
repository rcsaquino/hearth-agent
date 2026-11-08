//! The memory tool exposed to the model.

use std::sync::Arc;

use hearth_model::ToolSpec;
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};

use crate::service::{MemoryPatch, MemoryService, NewMemoryInput};
use crate::types::{MemoryError, MemoryItem, MemoryKind, SourceKind};

/// The provenance a run attaches to an explicit memory save.
#[derive(Debug, Clone)]
pub struct MemorySource {
    pub source_kind: SourceKind,
    pub source_ref: Option<String>,
}

impl MemorySource {
    /// A fact stated by the user in a concrete entry.
    pub fn user(source_ref: impl Into<String>) -> Self {
        Self {
            source_kind: SourceKind::User,
            source_ref: Some(source_ref.into()),
        }
    }
}

/// The model-facing memory library tool.
pub struct MemoryTool {
    service: Arc<MemoryService>,
    source: Option<MemorySource>,
}

impl MemoryTool {
    /// Build the tool over one memory service.
    pub fn new(service: Arc<MemoryService>) -> Self {
        Self {
            service,
            source: None,
        }
    }

    /// Build the tool with the provenance of the current run.
    pub fn with_source(service: Arc<MemoryService>, source: MemorySource) -> Self {
        Self {
            service,
            source: Some(source),
        }
    }
}

impl Tool for MemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "memory".to_owned(),
            description: "Save, search, read, update, forget, or pin durable facts, preferences, people, projects, and rules. This is a library for lasting knowledge, not scratch state."
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["search", "get", "save", "update", "forget", "pin", "stats", "as_of"]
                    },
                    "query": {"type": "string"},
                    "id": {"type": "string"},
                    "content": {"type": "string"},
                    "kind": {
                        "type": "string",
                        "enum": ["fact", "preference", "person", "project", "event", "episode", "rule"]
                    },
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "importance": {"type": "integer", "minimum": 0, "maximum": 100},
                    "supersedes": {"type": "string"},
                    "pinned": {"type": "boolean"},
                    "include_archived": {"type": "boolean"},
                    "at": {"type": "string"}
                },
                "required": ["action"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "action is required"))?;
        match action {
            "search" => {
                let query = required_str(arguments, "query")?;
                let include_archived = arguments
                    .get("include_archived")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let items = self
                    .service
                    .search_with(query, 10, include_archived)
                    .map_err(tool_error)?;
                Ok(ToolOutput {
                    content: render_items(&items),
                    is_error: false,
                })
            }
            "get" => {
                let id = required_str(arguments, "id")?;
                let item = self.service.get(id).map_err(tool_error)?.ok_or_else(|| {
                    ToolError::invalid("not_found", format!("memory {id} not found"))
                })?;
                Ok(ToolOutput {
                    content: render_one(&item),
                    is_error: false,
                })
            }
            "save" => {
                let content = required_str(arguments, "content")?;
                let kind = arguments
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(MemoryKind::parse)
                    .unwrap_or(MemoryKind::Fact);
                let importance = arguments
                    .get("importance")
                    .and_then(Value::as_i64)
                    .unwrap_or(50);
                let tags = tags(arguments);
                let supersedes = arguments
                    .get("supersedes")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let source_kind = self
                    .source
                    .as_ref()
                    .map(|source| source.source_kind)
                    .unwrap_or(SourceKind::Session);
                let source_ref = self
                    .source
                    .as_ref()
                    .and_then(|source| source.source_ref.clone());
                let item = self
                    .service
                    .save(NewMemoryInput {
                        content: content.to_owned(),
                        kind,
                        tags,
                        importance,
                        source_kind,
                        source_ref,
                        supersedes,
                        valid_from: None,
                        valid_to: None,
                    })
                    .map_err(tool_error)?;
                Ok(ToolOutput {
                    content: item.id,
                    is_error: false,
                })
            }
            "update" => {
                let id = required_str(arguments, "id")?;
                let patch = MemoryPatch {
                    content: arguments
                        .get("content")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    tags: arguments.get("tags").map(|_| tags(arguments)),
                    importance: arguments.get("importance").and_then(Value::as_i64),
                    ..MemoryPatch::default()
                };
                self.service.update(id, patch).map_err(tool_error)?;
                Ok(ToolOutput {
                    content: format!("updated {id}"),
                    is_error: false,
                })
            }
            "forget" => {
                let id = required_str(arguments, "id")?;
                self.service.forget(id).map_err(tool_error)?;
                Ok(ToolOutput {
                    content: format!("forgotten {id}"),
                    is_error: false,
                })
            }
            "pin" => {
                let id = required_str(arguments, "id")?;
                let pinned = arguments
                    .get("pinned")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                self.service.pin(id, pinned).map_err(tool_error)?;
                Ok(ToolOutput {
                    content: format!("{} {id}", if pinned { "pinned" } else { "unpinned" }),
                    is_error: false,
                })
            }
            "stats" => {
                let stats = self.service.stats().map_err(tool_error)?;
                Ok(ToolOutput {
                    content: json!({
                        "total": stats.total,
                        "by_status": stats.by_status,
                        "by_kind": stats.by_kind,
                    })
                    .to_string(),
                    is_error: false,
                })
            }
            "as_of" => {
                let at = required_str(arguments, "at")?;
                let at_micros = parse_at(at).ok_or_else(|| {
                    ToolError::invalid("invalid_arguments", "at must be RFC3339 or YYYY-MM-DD")
                })?;
                let query = arguments.get("query").and_then(Value::as_str);
                let include_archived = arguments
                    .get("include_archived")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let items = self
                    .service
                    .as_of_with(query, at_micros, include_archived)
                    .map_err(tool_error)?;
                Ok(ToolOutput {
                    content: render_items(&items),
                    is_error: false,
                })
            }
            other => Err(ToolError::invalid(
                "unknown_action",
                format!("unknown memory action: {other}"),
            )),
        }
    }
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::invalid("invalid_arguments", format!("{key} is required")))
}

fn tags(arguments: &Value) -> Vec<String> {
    arguments
        .get("tags")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn tool_error(error: MemoryError) -> ToolError {
    match error {
        MemoryError::Secret(_) => ToolError::invalid(
            "memory_secret",
            "content looks like a credential and was refused",
        ),
        MemoryError::NotFound(id) => {
            ToolError::invalid("not_found", format!("memory {id} not found"))
        }
        other => ToolError::invalid("memory_failed", other.to_string()),
    }
}

fn render_items(items: &[MemoryItem]) -> String {
    if items.is_empty() {
        return "no memories found.".to_owned();
    }
    items.iter().map(compact).collect::<Vec<_>>().join("\n")
}

fn compact(item: &MemoryItem) -> String {
    format!(
        "[{}] {}, {}, p={}: {}",
        item.id,
        item.kind.as_str(),
        format_date(item.created_at),
        item.importance,
        item.content
    )
}

fn render_one(item: &MemoryItem) -> String {
    json!({
        "id": item.id,
        "kind": item.kind.as_str(),
        "content": item.content,
        "tags": item.tags,
        "importance": item.importance,
        "status": item.status.as_str(),
        "standing": item.standing.as_str(),
        "source_kind": item.source_kind.as_str(),
        "source_ref": item.source_ref,
        "supersedes_id": item.supersedes_id,
        "created_at": item.created_at,
        "valid_from": item.valid_from,
        "valid_to": item.valid_to,
    })
    .to_string()
}

fn parse_at(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() == 10 {
        let days = parse_date(text)?;
        return Some(days * 86_400_000_000);
    }
    let (date_text, rest) = text.split_once('T').or_else(|| text.split_once(' '))?;
    let days = parse_date(date_text)?;
    let (clock, offset_seconds) = if let Some(clock) = rest.strip_suffix('Z') {
        (clock, 0)
    } else if let Some((clock, offset)) = rest.split_once('+') {
        (clock, parse_offset(offset)?)
    } else if let Some((clock, offset)) = rest.rsplit_once('-') {
        (clock, -parse_offset(offset)?)
    } else {
        (rest, 0)
    };
    let mut parts = clock.split(':');
    let hours: i64 = parts.next()?.parse().ok()?;
    let minutes: i64 = parts.next().unwrap_or("0").parse().ok()?;
    let seconds: i64 = parts.next().unwrap_or("0").parse().ok()?;
    if !(0..24).contains(&hours) || !(0..60).contains(&minutes) || !(0..60).contains(&seconds) {
        return None;
    }
    Some(
        days * 86_400_000_000 + (hours * 3600 + minutes * 60 + seconds) * 1_000_000
            - offset_seconds * 1_000_000,
    )
}

fn parse_offset(text: &str) -> Option<i64> {
    let mut parts = text.split(':');
    let hours: i64 = parts.next()?.parse().ok()?;
    let minutes: i64 = parts.next().unwrap_or("0").parse().ok()?;
    Some(hours * 3600 + minutes * 60)
}

fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn format_date(micros: i64) -> String {
    let days = micros.div_euclid(86_400_000_000);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year + i64::from(month <= 2);
    (year, month as u32, day as u32)
}
