//! The memory tools.
//!
//! Four verbs, not one dispatcher. Each does one thing to one kind of object, so
//! the model never has to choose a store, remember an id prefix, or discover that
//! a parameter was wrong for a different mode. That is worth more than the
//! handful of tokens a combined schema would save: a tool the model uses
//! correctly on the first call is the cheapest tool there is.
//!
//! The write result carries one extra signal. When a new statement lands beside
//! an existing one about the same subject, the reply names the neighbour and its
//! overlap, so the model can replace it on the same turn instead of leaving two
//! contradictory facts for a later question to trip over.

use std::sync::Arc;

use hearth_model::ToolSpec;
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};

use crate::errors::{Code, MemoryError};
use crate::model::{Kind, Memory, NewMemory};
use crate::{MemoryService, digest, store};

pub const SAVE: &str = "memory_save";
pub const RECALL: &str = "memory_recall";
pub const FORGET: &str = "memory_forget";
pub const TRANSCRIPT: &str = "memory_transcript";

/// Everything the tools need.
///
/// `source` is the entry this run's writes came from. It lives on the tool set
/// rather than on the service, because the service is shared with the helper
/// pool, the scheduler, and compaction, and a service-wide slot would stamp their
/// writes with the user's entry too.
#[derive(Clone)]
pub struct ToolMemory {
    pub service: MemoryService,
    pub source: Option<String>,
}

/// Build the four tools.
pub fn build(memory: &ToolMemory) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ForgetTool {
            memory: memory.clone(),
        }),
        Arc::new(RecallTool {
            memory: memory.clone(),
        }),
        Arc::new(SaveTool {
            memory: memory.clone(),
        }),
        Arc::new(TranscriptTool {
            memory: memory.clone(),
        }),
    ]
}

fn failure(error: MemoryError) -> ToolError {
    match error.code {
        Code::Unavailable => ToolError::temporary(error.code.as_str(), error.message),
        _ => ToolError::invalid(error.code.as_str(), error.message),
    }
}

fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        parameters,
    }
}

fn text(content: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: content.into(),
        is_error: false,
    }
}

fn required(value: &Value, name: &str) -> Result<String, ToolError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ToolError::invalid("invalid_input", format!("{name} is required.")))
}

fn optional(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn optional_flag(value: &Value, name: &str) -> bool {
    value.get(name).and_then(Value::as_bool).unwrap_or(false)
}

fn limit(value: &Value, default: i64) -> i64 {
    value
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(default)
        .clamp(1, 50)
}

struct SaveTool {
    memory: ToolMemory,
}

impl Tool for SaveTool {
    fn spec(&self) -> ToolSpec {
        spec(
            SAVE,
            "Store one durable statement about the user or their world: a preference, a person, \
             a decision, a commitment, or a fact. Use it when the user says something lasting. \
             Give `subject` the person, project, or thing the statement is about, because that \
             is what makes it findable later. To replace something already stored, pass its id \
             as `supersedes`; the old statement is kept as history. Pin only a rule that must be \
             obeyed on every turn.",
            json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "One statement, in the user's terms."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["rule", "preference", "person", "decision", "commitment", "episode", "fact"],
                        "description": "Defaults to fact."
                    },
                    "subject": {
                        "type": "string",
                        "description": "The person, project, or thing it is about, such as alice or or board."
                    },
                    "supersedes": {
                        "type": "string",
                        "description": "The id of the statement this one replaces."
                    },
                    "pin": {
                        "type": "boolean",
                        "description": "Injects a rule into every request. Only for kind rule."
                    }
                },
                "required": ["content"]
            }),
        )
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let content = required(arguments, "content")?;
        let kind = match optional(arguments, "kind") {
            Some(value) => value
                .parse::<Kind>()
                .map_err(|error| ToolError::invalid(error.code.as_str(), error.message))?,
            None => Kind::Fact,
        };
        let mut item = NewMemory::new(content.clone()).kind(kind);
        if let Some(subject) = optional(arguments, "subject") {
            item = item.subject(subject);
        }
        if let Some(previous) = optional(arguments, "supersedes") {
            item = item.supersedes(previous);
        }
        if optional_flag(arguments, "pin") {
            item = item.pinned(true);
        }
        let item = match &self.memory.source {
            Some(entry) => item.from_entry(Some(entry.clone())),
            None => item,
        };
        let outcome = self.memory.service.write(item).map_err(failure)?;
        let now = self.memory.service.session().now_micros();
        let memory = outcome.memory();
        if !outcome.created() {
            return Ok(text(format!(
                "already stored as {}: {}",
                memory.id, memory.content
            )));
        }
        let mut reply = format!(
            "saved {}{}{}",
            memory.id,
            if memory.pinned { " pinned" } else { "" },
            if memory.subject.is_empty() {
                String::new()
            } else {
                format!(" about {}", memory.subject)
            }
        );
        if let Some((neighbour, overlap)) = store::related(
            self.memory.service.session(),
            &memory.subject,
            &memory.content,
            &memory.id,
        )
        .map_err(failure)?
        {
            reply.push_str(&format!(
                "\nrelated: {} ({:.2} overlap) {}. Pass supersedes=\"{}\" if this replaces it.",
                neighbour.id,
                overlap,
                digest::line(&neighbour, now),
                neighbour.id
            ));
        }
        Ok(text(reply))
    }
}

struct RecallTool {
    memory: ToolMemory,
}

impl Tool for RecallTool {
    fn spec(&self) -> ToolSpec {
        spec(
            RECALL,
            "Search memory. With no arguments it lists the newest statements. `id` returns one \
             statement in full. `subject` returns everything about a person, project, or thing. \
             `as_of` reads what was true at that instant. Use it before answering a question \
             about the user's world, and whenever the recall digest says more exists.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keywords to search for."
                    },
                    "id": {
                        "type": "string",
                        "description": "One statement's id."
                    },
                    "subject": {
                        "type": "string",
                        "description": "Everything about this person, project, or thing."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["rule", "preference", "person", "decision", "commitment", "episode", "fact"]
                    },
                    "as_of": {
                        "type": "string",
                        "description": "RFC3339 or YYYY-MM-DD. Reads what was true then."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "At most 50. Defaults to 20."
                    }
                }
            }),
        )
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let at = match optional(arguments, "as_of") {
            Some(value) => Some(parse_instant(&value)?),
            None => None,
        };
        let count = limit(arguments, 20);
        let now = self.memory.service.session().now_micros();

        if let Some(id) = optional(arguments, "id") {
            let memory = self
                .memory
                .service
                .get(&id)
                .map_err(failure)?
                .ok_or_else(|| {
                    ToolError::invalid("not_found", format!("memory {id} does not exist."))
                })?;
            let subject = if memory.subject.is_empty() {
                "no subject".to_owned()
            } else {
                memory.subject.clone()
            };
            let history = if memory.is_current() {
                String::new()
            } else {
                format!(
                    "\nreplaced by {}",
                    memory
                        .superseded_by
                        .as_deref()
                        .unwrap_or("a newer statement")
                )
            };
            return Ok(text(format!(
                "{} [{}, {}, {}]: {}\n{}{}",
                memory.id,
                memory.kind,
                subject,
                if memory.pinned { "pinned" } else { "unpinned" },
                memory.content,
                format_args!(
                    "source {} since {}",
                    memory.source.as_str(),
                    memory.valid_from
                ),
                history
            )));
        }

        let kind_filter = match optional(arguments, "kind") {
            Some(value) => Some(
                value
                    .parse::<Kind>()
                    .map_err(|error| ToolError::invalid(error.code.as_str(), error.message))?,
            ),
            None => None,
        };

        let found = if let Some(subject) = optional(arguments, "subject") {
            self.memory
                .service
                .subject(&subject, at, count)
                .map_err(failure)?
        } else if let Some(query) = optional(arguments, "query") {
            self.memory
                .service
                .search(&query, at, count)
                .map_err(failure)?
        } else {
            self.memory.service.recent(count).map_err(failure)?
        };

        let selected: Vec<&Memory> = found
            .iter()
            .filter(|memory| kind_filter.is_none_or(|kind| memory.kind == kind))
            .collect();
        if selected.is_empty() {
            return Ok(text("no memories matched."));
        }
        let mut lines: Vec<String> = selected
            .iter()
            .map(|memory| digest::line(memory, now))
            .collect();
        let hidden = found.len().saturating_sub(selected.len());
        if hidden > 0 {
            lines.push(format!("{hidden} other kinds were not shown."));
        }
        Ok(text(lines.join("\n")))
    }
}

/// Parse an RFC3339 instant or a date.
fn parse_instant(value: &str) -> Result<i64, ToolError> {
    if let Ok(instant) =
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
    {
        return Ok(instant.unix_timestamp_nanos().div_euclid(1_000) as i64);
    }
    let date = time::Date::parse(
        value,
        time::macros::format_description!("[year]-[month]-[day]"),
    )
    .map_err(|_| {
        ToolError::invalid(
            "invalid_input",
            "as_of must be an RFC3339 instant or a YYYY-MM-DD date.",
        )
    })?;
    Ok(date
        .midnight()
        .assume_utc()
        .unix_timestamp_nanos()
        .div_euclid(1_000) as i64)
}

struct ForgetTool {
    memory: ToolMemory,
}

impl Tool for ForgetTool {
    fn spec(&self) -> ToolSpec {
        spec(
            FORGET,
            "Delete one statement permanently. Prefer `memory_save` with `supersedes` when a fact \
             changed, because the old version is worth keeping as history. Delete when the \
             statement was wrong, private, or never true.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The statement's id."
                    }
                },
                "required": ["id"]
            }),
        )
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let id = required(arguments, "id")?;
        self.memory.service.forget(&id).map_err(failure)?;
        Ok(text(format!("forgotten {id}")))
    }
}

struct TranscriptTool {
    memory: ToolMemory,
}

impl Tool for TranscriptTool {
    fn spec(&self) -> ToolSpec {
        spec(
            TRANSCRIPT,
            "Search the saved conversation record, or read one entry by its conversation#seq \
             reference. Use it for what was literally said in an earlier conversation.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Words to search for."
                    },
                    "reference": {
                        "type": "string",
                        "description": "A conversation#seq citation."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "At most 50. Defaults to 10."
                    }
                }
            }),
        )
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        if let Some(reference) = optional(arguments, "reference") {
            let entry = self
                .memory
                .service
                .entry(&reference)
                .map_err(failure)?
                .ok_or_else(|| {
                    ToolError::invalid("not_found", format!("{reference} is not a saved entry."))
                })?;
            // A stored citation instant, not a personal timestamp, so keep it in UTC.
            return Ok(text(format!(
                "{}#{} {} {}\n{}",
                entry.conversation_id,
                entry.seq,
                entry.kind,
                hearth_core::Zone::utc().iso(entry.created_at),
                entry.text
            )));
        }
        let query = required(arguments, "query")?;
        let hits = self
            .memory
            .service
            .transcript(&query, limit(arguments, 10))
            .map_err(failure)?;
        if hits.is_empty() {
            return Ok(text("no saved entries matched."));
        }
        let lines: Vec<String> = hits
            .iter()
            .map(|hit| format!("{} [{}] {}", hit.reference(), hit.kind, hit.snippet))
            .collect();
        Ok(text(lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearth_core::{Clock, Harness, Save, UuidV7};
    use tempfile::TempDir;

    struct FixedClock(i64);

    impl Clock for FixedClock {
        fn now_micros(&self) -> i64 {
            self.0
        }
    }

    fn memory() -> (TempDir, MemoryService) {
        let directory = TempDir::new().expect("temp home");
        let harness = Harness::open(directory.path()).expect("harness");
        (directory, MemoryService::new(harness.session().clone()))
    }

    fn registry(service: &MemoryService) -> hearth_tools::ToolRegistry {
        hearth_tools::ToolRegistry::new(service.tools())
    }

    #[test]
    fn a_saved_statement_can_be_recalled_by_id() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = tools
            .call(
                SAVE,
                &json!({"content": "Alice manages the OR board.", "subject": "alice"}),
            )
            .expect("call")
            .expect("result");
        assert!(saved.content.starts_with("saved m_"), "{}", saved.content);
        let id = saved.content.split_whitespace().nth(1).expect("id");
        let recalled = tools
            .call(RECALL, &json!({"id": id}))
            .expect("call")
            .expect("result");
        assert!(
            recalled.content.contains("OR board"),
            "{}",
            recalled.content
        );
    }

    #[test]
    fn a_statement_beside_a_related_one_names_the_neighbour() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        tools
            .call(
                SAVE,
                &json!({"content": "the primary database is Postgres", "subject": "primary database"}),
            )
            .expect("call")
            .expect("result");
        let second = tools
            .call(
                SAVE,
                &json!({"content": "the primary database is SQLite now", "subject": "primary database"}),
            )
            .expect("call")
            .expect("result");
        assert!(
            second.content.contains("Pass supersedes"),
            "the write must teach the model to replace: {}",
            second.content
        );
    }

    #[test]
    fn a_replaced_statement_is_kept_as_history() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let first = tools
            .call(
                SAVE,
                &json!({"content": "the lab is in building 4", "subject": "lab"}),
            )
            .expect("call")
            .expect("result");
        let id = first
            .content
            .split_whitespace()
            .nth(1)
            .expect("id")
            .to_owned();
        tools
            .call(
                SAVE,
                &json!({"content": "the lab is in building 9", "subject": "lab", "supersedes": id}),
            )
            .expect("call")
            .expect("result");

        let current = tools
            .call(RECALL, &json!({"query": "where is the lab"}))
            .expect("call")
            .expect("result");
        assert!(
            current.content.contains("building 9"),
            "{}",
            current.content
        );
        assert!(
            !current.content.contains("building 4"),
            "a replaced statement must not answer a current question: {}",
            current.content
        );

        let historical = tools
            .call(RECALL, &json!({"id": id}))
            .expect("call")
            .expect("result");
        assert!(
            historical.content.contains("building 4"),
            "{}",
            historical.content
        );
        assert!(
            historical.content.contains("replaced by"),
            "{}",
            historical.content
        );
    }

    #[test]
    fn forgetting_a_missing_statement_says_so() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let error = tools
            .call(FORGET, &json!({"id": "m_missing"}))
            .expect("call")
            .expect_err("missing");
        assert!(error.message.contains("does not exist"), "{error:?}");
    }

    #[test]
    fn a_transcript_citation_is_rendered_in_the_configured_zone() {
        let directory = TempDir::new().expect("temp home");
        let instant = 1_768_438_800_000_000; // 2026-01-15T01:00:00Z
        let harness = Harness::open_with(
            directory.path(),
            Arc::new(FixedClock(instant)),
            Arc::new(UuidV7),
        )
        .expect("harness");
        let session = harness.session().clone();
        let conversation = session
            .create_conversation("Test", "local", "local:zone")
            .expect("conversation");
        session
            .save(Save::new(&conversation.id).entry("user", json!("the OR board meets on Thursdays")))
            .expect("entry");
        let service = MemoryService::new(session);
        let tools = hearth_tools::ToolRegistry::new(
            service.tools_at(None, hearth_core::Zone::fixed(8 * 3600)),
        );
        let reply = tools
            .call(
                TRANSCRIPT,
                &json!({"reference": format!("{}#1", conversation.id)}),
            )
            .expect("call")
            .expect("result");
        assert!(
            reply.content.contains("2026-01-15T09:00:00+08:00"),
            "{}",
            reply.content
        );
    }

    #[test]
    fn a_greeting_is_refused_as_content() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let error = tools
            .call(SAVE, &json!({"content": "   "}))
            .expect("call")
            .expect_err("empty");
        assert_eq!(error.code, "invalid_input");
    }
}
