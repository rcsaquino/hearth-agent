//! The memory tool.
//!
//! One tool with four actions: `save`, `recall`, `forget`, and `transcript`.
//! The shape mirrors `files`, so the model has one name and one schema to read.
//! `action` is required, the related fields are shared, and each action names
//! the fields it uses.
//!
//! The save result carries one extra signal. When a new statement lands beside
//! an existing one about the same subject, the reply names the neighbour and its
//! overlap, so the model can replace it on the same turn instead of leaving two
//! contradictory facts for a later question to trip over.

use std::sync::Arc;

use hearth_core::Zone;
use hearth_model::ToolSpec;
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};

use crate::errors::{Code, MemoryError};
use crate::model::{Kind, Memory, NewMemory};
use crate::{MemoryService, digest, store};

/// The tool name.
pub const MEMORY: &str = "memory";
/// Store one statement.
pub const SAVE: &str = "save";
/// Search or read statements.
pub const RECALL: &str = "recall";
/// Delete one statement permanently.
pub const FORGET: &str = "forget";
/// Search the saved conversation record.
pub const TRANSCRIPT: &str = "transcript";

/// Everything the tool needs.
///
/// `source` is the entry this run's writes came from. It lives on the tool set
/// rather than on the service, because the service is shared with the helper
/// pool, the scheduler, and compaction, and a service-wide slot would stamp their
/// writes with the user's entry too.
#[derive(Clone)]
pub struct ToolMemory {
    pub service: MemoryService,
    pub source: Option<String>,
    pub zone: Zone,
}

/// Build the memory tool.
pub fn build(memory: &ToolMemory) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(MemoryTool {
        memory: memory.clone(),
    })]
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

struct MemoryTool {
    memory: ToolMemory,
}

impl Tool for MemoryTool {
    fn spec(&self) -> ToolSpec {
        spec(
            MEMORY,
            "Read or change memory. `action` selects the operation: `save` stores one durable \
             statement, `recall` searches or reads by id, subject, or time, `forget` deletes one \
             statement permanently, and `transcript` searches or reads the saved conversation
             record.",
            json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [SAVE, RECALL, FORGET, TRANSCRIPT]
                    },
                    "content": {
                        "type": "string",
                        "description": "For save. One standalone statement of at most 200 characters, understandable without prior context. Clarify references when necessary to avoid ambiguity, but do not add unnecessary details."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["rule", "preference", "person", "decision", "commitment", "episode", "fact"],
                        "description": "For save or recall. On save it defaults to fact. On recall it filters."
                    },
                    "subject": {
                        "type": "string",
                        "description": "For save or recall. The person, project, or thing it is about, such as alice or the board."
                    },
                    "supersedes": {
                        "type": "string",
                        "description": "For save. The id of the statement this one replaces."
                    },
                    "pin": {
                        "type": "boolean",
                        "description": "For save. Injects a rule into every request. Only for kind rule."
                    },
                    "query": {
                        "type": "string",
                        "description": "For recall or transcript. Words to search for."
                    },
                    "id": {
                        "type": "string",
                        "description": "For recall or forget. One statement's id, such as m_0123456789. Case does not matter."
                    },
                    "as_of": {
                        "type": "string",
                        "description": "For recall. RFC3339 or YYYY-MM-DD. Reads what was true then; a bare date means that whole local day."
                    },
                    "reference": {
                        "type": "string",
                        "description": "For transcript. A conversation#seq citation."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "For recall or transcript. At most 50. Recall defaults to 20 and transcript to 10."
                    }
                },
                "required": ["action"]
            }),
        )
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let action = required(arguments, "action")?;
        match action.as_str() {
            SAVE => self.save(arguments),
            RECALL => self.recall(arguments),
            FORGET => self.forget(arguments),
            TRANSCRIPT => self.transcript(arguments),
            other => Err(ToolError::invalid(
                "unknown_action",
                format!("unknown memory action: {other}"),
            )),
        }
    }
}

impl MemoryTool {
    fn save(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
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
                "\nrelated: {} ({:.2} overlap) {}. This is a separate statement, not a duplicate.",
                neighbour.id,
                overlap,
                digest::tool_line(&neighbour, now)
            ));
        }
        Ok(text(reply))
    }

    fn recall(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let at = match optional(arguments, "as_of") {
            Some(value) => Some(parse_instant(&value, &self.memory.zone)?),
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
                    self.memory.zone.iso(memory.valid_from)
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
                .subject_of_kind(&subject, at, kind_filter, count)
                .map_err(failure)?
        } else if let Some(query) = optional(arguments, "query") {
            self.memory
                .service
                .search_of_kind(&query, at, kind_filter, count)
                .map_err(failure)?
        } else {
            self.memory
                .service
                .recent_of_kind(at, kind_filter, count)
                .map_err(failure)?
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
            .map(|memory| digest::tool_line(memory, now))
            .collect();
        let hidden = found.len().saturating_sub(selected.len());
        if hidden > 0 {
            lines.push(format!("{hidden} other kinds were not shown."));
        }
        Ok(text(lines.join("\n")))
    }

    fn forget(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let id = required(arguments, "id")?;
        self.memory.service.forget(&id).map_err(failure)?;
        Ok(text(format!("forgotten {id}")))
    }

    fn transcript(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        if let Some(reference) = optional(arguments, "reference") {
            let entry = self
                .memory
                .service
                .entry(&reference)
                .map_err(failure)?
                .ok_or_else(|| {
                    ToolError::invalid("not_found", format!("{reference} is not a saved entry."))
                })?;
            return Ok(text(format!(
                "{}#{} {} {}\n{}",
                entry.conversation_id,
                entry.seq,
                entry.kind,
                self.memory.zone.iso(entry.created_at),
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

/// Parse an RFC3339 instant or a date.
fn parse_instant(value: &str, zone: &Zone) -> Result<i64, ToolError> {
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
    let utc_midnight = date
        .midnight()
        .assume_utc()
        .unix_timestamp_nanos()
        .div_euclid(1_000) as i64;
    // The date names the whole local day, and the store predicate is
    // start-inclusive and end-exclusive, so the boundary is the next day's start.
    Ok(date
        .next_day()
        .and_then(|next| {
            let year = i16::try_from(next.year()).ok()?;
            zone.day_start(year, next.month().into(), next.day())
        })
        .unwrap_or(utc_midnight))
}

#[cfg(test)]
fn call(
    tools: &hearth_tools::ToolRegistry,
    action: &str,
    mut arguments: Value,
) -> Result<hearth_tools::ToolOutput, hearth_tools::ToolError> {
    arguments["action"] = json!(action);
    tools
        .call(MEMORY, &arguments)
        .expect("the memory tool is registered")
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
    fn the_memory_tool_is_one_tool_with_four_actions() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let spec = tools
            .specs()
            .into_iter()
            .find(|spec| spec.name == MEMORY)
            .expect("the memory tool");
        assert_eq!(spec.parameters["required"], json!(["action"]));
        assert_eq!(
            spec.parameters["properties"]["action"]["enum"],
            json!([SAVE, RECALL, FORGET, TRANSCRIPT])
        );
    }

    #[test]
    fn the_memory_tool_requires_an_action() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let error = tools
            .call(MEMORY, &json!({"content": "a statement"}))
            .expect("call")
            .expect_err("action is required");
        assert!(error.message.contains("action is required"), "{error:?}");
    }

    #[test]
    fn an_unknown_memory_action_is_refused() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let error = tools
            .call(MEMORY, &json!({"action": "rename"}))
            .expect("call")
            .expect_err("unknown action");
        assert_eq!(error.code, "unknown_action");
        assert!(error.message.contains("rename"), "{error:?}");
    }

    #[test]
    fn a_kind_filter_applies_before_the_limit() {
        let (_directory, service) = memory();
        service
            .write(NewMemory::new("never deploy on friday").kind(Kind::Rule))
            .expect("rule");
        for index in 0..30 {
            service
                .write(NewMemory::new(format!("fact number {index}")).kind(Kind::Fact))
                .expect("fact");
        }
        let tools = registry(&service);

        let reply = call(&tools, RECALL, json!({"kind": "rule"})).expect("result");

        assert!(
            reply.content.contains("never deploy on friday"),
            "a kind filter must not be hidden behind newer rows: {}",
            reply.content
        );
    }

    #[test]
    fn a_recall_line_keeps_its_id_for_addressing() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "Alice manages the OR board.", "subject": "alice"}),
        )
        .expect("result");
        let id = saved.content.split_whitespace().nth(1).expect("id");
        let recalled =
            call(&tools, RECALL, json!({"query": "who runs the OR board"})).expect("result");
        let row = recalled
            .content
            .lines()
            .find(|line| line.contains("OR board"))
            .expect("the recalled row");
        assert!(row.contains(" | "), "{row}");
        assert!(row.ends_with(id), "the recall tool must keep the id: {row}");
    }

    #[test]
    fn a_saved_statement_can_be_recalled_by_id() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "Alice manages the OR board.", "subject": "alice"}),
        )
        .expect("result");
        assert!(saved.content.starts_with("saved m_"), "{}", saved.content);
        let id = saved.content.split_whitespace().nth(1).expect("id");
        let recalled = call(&tools, RECALL, json!({"id": id})).expect("result");
        assert!(
            recalled.content.contains("OR board"),
            "{}",
            recalled.content
        );
    }

    #[test]
    fn a_recall_by_id_accepts_case_and_whitespace() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "Alice manages the OR board.", "subject": "alice"}),
        )
        .expect("result");
        let id = saved.content.split_whitespace().nth(1).expect("id");
        let typed = format!("  {}  ", id.to_uppercase());
        let recalled = call(&tools, RECALL, json!({"id": typed})).expect("result");
        assert!(
            recalled.content.contains("OR board"),
            "{}",
            recalled.content
        );
    }

    #[test]
    fn a_forget_by_id_accepts_case_and_whitespace() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "Alice manages the OR board.", "subject": "alice"}),
        )
        .expect("result");
        let id = saved.content.split_whitespace().nth(1).expect("id");
        let typed = format!("  {}  ", id.to_uppercase());
        call(&tools, FORGET, json!({"id": typed})).expect("result");
        let error = call(&tools, RECALL, json!({"id": id})).expect_err("the row is gone");
        assert!(error.message.contains("does not exist"), "{error:?}");
    }

    #[test]
    fn a_pronoun_is_accepted_by_the_save_tool() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "He likes apples", "subject": "bob"}),
        )
        .expect("a pronoun stores like any other word");
        assert!(saved.content.starts_with("saved m_"), "{}", saved.content);
        assert_eq!(service.count().expect("count"), 1);
    }

    #[test]
    fn a_named_actor_is_accepted_by_the_save_tool() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        for (content, subject) in [
            ("Bob likes apples", "bob"),
            ("the user likes apples", "user"),
        ] {
            call(
                &tools,
                SAVE,
                json!({"content": content, "subject": subject}),
            )
            .expect("a named actor is standalone");
        }
        assert_eq!(service.count().expect("count"), 2);
    }

    #[test]
    fn a_first_person_save_is_stored() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(&tools, SAVE, json!({"content": "I prefer dark mode"}))
            .expect("first person stores as written");
        assert!(saved.content.starts_with("saved m_"), "{}", saved.content);
    }

    #[test]
    fn a_third_person_statement_stores_whatever_its_subject() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "she manages the OR board", "subject": "alice"}),
        )
        .expect("the subject does not change what content is allowed");
        assert!(saved.content.starts_with("saved m_"), "{}", saved.content);
    }

    #[test]
    fn a_rule_is_stored_pinned() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "Never deploy on Friday", "kind": "rule", "pin": true}),
        )
        .expect("an imperative rule is standalone");
        assert!(saved.content.contains("pinned"), "{}", saved.content);
    }

    fn content_description(service: &MemoryService) -> String {
        let spec = service
            .tools()
            .iter()
            .find(|tool| tool.spec().name == MEMORY)
            .expect("save tool")
            .spec();
        spec.parameters["properties"]["content"]["description"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn the_save_schema_asks_for_a_statement_that_stands_alone() {
        let (_directory, service) = memory();
        let description = content_description(&service);
        assert!(description.contains("standalone"), "{description}");
        assert!(
            description.contains("without prior context"),
            "the schema must say a statement is read with no context: {description}"
        );
        assert!(
            description.contains("ambiguity"),
            "the schema must ask for references to be clarified: {description}"
        );
        assert!(
            description.contains("unnecessary details"),
            "the schema must forbid padding: {description}"
        );
        assert!(
            !description.contains("pronoun"),
            "the schema takes no position on pronouns: {description}"
        );
        assert!(
            !description.contains("actor"),
            "the schema no longer asks for a named actor: {description}"
        );
    }

    #[test]
    fn a_statement_beside_a_related_one_names_the_neighbour() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        call(
            &tools,
            SAVE,
            json!({"content": "the primary database is Postgres", "subject": "primary database"}),
        )
        .expect("result");
        let second = call(
            &tools,
            SAVE,
            json!({"content": "the primary database is SQLite now", "subject": "primary database"}),
        )
        .expect("result");
        assert!(
            second.content.contains("related"),
            "a neighbour must be named: {}",
            second.content
        );
        assert!(
            second.content.contains("supersedes ="),
            "the reply must name the replacement lever, because a changed value is a \
             replacement the model has to make on this turn: {}",
            second.content
        );
    }

    #[test]
    fn a_subjectless_statement_beside_a_related_one_names_the_neighbour() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        call(
            &tools,
            SAVE,
            json!({"content": "Alice manages the OR board", "subject": "alice"}),
        )
        .expect("result");
        let second = call(
            &tools,
            SAVE,
            json!({"content": "the OR board needs two approvals"}),
        )
        .expect("result");
        assert!(
            second.content.contains("related"),
            "a subjectless write must still name its neighbour: {}",
            second.content
        );
    }

    #[test]
    fn a_bare_as_of_date_means_the_local_day() {
        let (_directory, service) = memory();
        service
            .write(
                NewMemory::new("the release gate needs two approvals")
                    .subject("release")
                    .valid_from(1_768_454_800_000_000),
            )
            .expect("first");
        service
            .write(
                NewMemory::new("the release gate moved to the deploy bot")
                    .subject("release")
                    .valid_from(1_768_512_400_000_000),
            )
            .expect("second");
        let tools = hearth_tools::ToolRegistry::new(service.tools_at(None, Zone::fixed(8 * 3600)));
        let reply = call(
            &tools,
            RECALL,
            json!({"subject": "release", "as_of": "2026-01-15"}),
        )
        .expect("result");
        assert!(
            reply
                .content
                .contains("the release gate needs two approvals"),
            "{}",
            reply.content
        );
        assert!(
            !reply
                .content
                .contains("the release gate moved to the deploy bot"),
            "{}",
            reply.content
        );
    }

    #[test]
    fn a_replaced_statement_is_kept_as_history() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let first = call(
            &tools,
            SAVE,
            json!({"content": "the lab is in building 4", "subject": "lab"}),
        )
        .expect("result");
        let id = first
            .content
            .split_whitespace()
            .nth(1)
            .expect("id")
            .to_owned();
        call(
            &tools,
            SAVE,
            json!({"content": "the lab is in building 9", "subject": "lab", "supersedes": id}),
        )
        .expect("result");

        let current = call(&tools, RECALL, json!({"query": "where is the lab"})).expect("result");
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

        let historical = call(&tools, RECALL, json!({"id": id})).expect("result");
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
        let error = call(&tools, FORGET, json!({"id": "m_missing"})).expect_err("missing");
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
            .save(
                Save::new(&conversation.id).entry("user", json!("the OR board meets on Thursdays")),
            )
            .expect("entry");
        let service = MemoryService::new(session);
        let tools = hearth_tools::ToolRegistry::new(
            service.tools_at(None, hearth_core::Zone::fixed(8 * 3600)),
        );
        let reply = call(
            &tools,
            TRANSCRIPT,
            json!({"reference": format!("{}#1", conversation.id)}),
        )
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
        let error = call(&tools, SAVE, json!({"content": "   "})).expect_err("empty");
        assert_eq!(error.code, "invalid_input");
    }

    #[test]
    fn the_save_schema_names_the_content_limit() {
        let (_directory, service) = memory();
        let description = content_description(&service);
        assert!(
            description.contains(&crate::store::CONTENT_LIMIT.to_string()),
            "{description}"
        );
    }

    #[test]
    fn a_long_statement_is_refused_with_its_length_and_the_limit() {
        let (_directory, service) = memory();
        let tools = registry(&service);
        let length = crate::store::CONTENT_LIMIT + 1;
        let error =
            call(&tools, SAVE, json!({"content": "a".repeat(length)})).expect_err("too long");
        assert_eq!(error.code, "invalid_input");
        assert!(
            error.message.contains(&length.to_string()),
            "{}",
            error.message
        );
        assert!(
            error
                .message
                .contains(&crate::store::CONTENT_LIMIT.to_string()),
            "{}",
            error.message
        );
    }
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;
    use hearth_core::Harness;
    use tempfile::TempDir;

    #[test]
    fn a_recall_by_id_prints_a_zoned_instant() {
        let directory = TempDir::new().expect("temp home");
        let harness = Harness::open(directory.path()).expect("harness");
        let service = MemoryService::new(harness.session().clone());
        let tools = hearth_tools::ToolRegistry::new(service.tools());
        let saved = call(
            &tools,
            SAVE,
            json!({"content": "the OR board meets on Tuesdays", "subject": "or board"}),
        )
        .expect("result");
        let id = saved.content.split_whitespace().nth(1).expect("id");

        let recalled = call(&tools, RECALL, json!({"id": id})).expect("result");
        let since = recalled
            .content
            .split("since ")
            .nth(1)
            .expect("a since stamp")
            .split_whitespace()
            .next()
            .expect("the stamp");
        assert!(
            since.len() == 25 && since.ends_with("+00:00"),
            "the stamp must be ISO 8601 with an offset: {since}"
        );
    }
}

#[cfg(test)]
mod as_of_tests {
    use super::*;
    use hearth_core::Harness;
    use tempfile::TempDir;

    #[test]
    fn a_bare_as_of_read_returns_what_was_true_then() {
        let directory = TempDir::new().expect("temp home");
        let harness = Harness::open(directory.path()).expect("harness");
        let session = harness.session();
        let service = MemoryService::new(session.clone());
        let now = session.now_micros();
        let old = now - 365 * 86_400 * 1_000_000;
        let earlier = service
            .write(
                NewMemory::new("the primary database is Postgres")
                    .subject("primary database")
                    .valid_from(old),
            )
            .expect("write")
            .memory()
            .clone();
        service
            .write(
                NewMemory::new("the primary database is SQLite now")
                    .subject("primary database")
                    .valid_from(now)
                    .supersedes(&earlier.id),
            )
            .expect("replace");

        let tools = hearth_tools::ToolRegistry::new(service.tools());
        let as_of = Zone::utc().iso(old + 5_000_000);
        let recalled = call(&tools, RECALL, json!({"as_of": as_of})).expect("result");
        assert!(
            recalled.content.contains("Postgres"),
            "the as-of read must return the old statement: {}",
            recalled.content
        );
        assert!(
            !recalled.content.contains("SQLite now"),
            "the as-of read must not return today's statement: {}",
            recalled.content
        );
    }
}
