//! The nightly skill review and its scheduler handler.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use hearth_core::{CoreError, Entry, NewUsage, Session, SubmissionStatus, parse_blocks};
use hearth_memory::{MemoryError, MemoryService};
use hearth_model::{
    ChatMessage, ChatRequest, ContentBlock, Pricing, Provider, Role, StreamEvent, Usage,
};
use serde_json::{Value, json};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::maintenance::{
    SKILL_REVIEW_HOUR, SKILL_REVIEW_MINUTE, local_offset, next_daily_micros, rfc3339,
};
use crate::memory_extract::entry_text;
use crate::scheduler::{TaskHandler, TaskHandlerError};
use crate::skills::{Skill, SkillError, SkillLibrary, SkillSave, SkillScope, SkillStatus};

const MAX_CHANGES: usize = 3;
const VIEW_TOKEN_CAP: i64 = 3000;
const SKILL_BODY_TOKEN_CAP: i64 = 4000;
const FIRST_REVIEW_DAYS: i64 = 30;

const INSTRUCTION: &str = "You review Hearth's skill library. Return valid JSON only: \
{\"changes\":[...]}. At most three changes. Each change is \
{\"action\":\"create\"|\"revise\"|\"promote\"|\"quarantine\"|\"archive\",\
\"name\":\"...\",\"description\":\"...\",\"body\":\"...\",\"evidence\":[\"entry-id\"],\
\"reason\":\"...\"}. Every change must cite entry ids from the recent successful runs below. \
A skill's own body is never evidence. Never edit a user-made skill. Create only when a workflow \
succeeded at least three times in the recent runs. Revise only with a clearly better method from \
a later run. Promote a draft after one successful use. Quarantine after two consecutive failures. \
Archive after 90 days unused or a failure rate above half after four uses. The body needs When to \
use, Steps, and Pitfalls sections. Do not store secrets.";

/// The outcome of one review pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewReport {
    pub applied: usize,
    pub rejected: usize,
}

/// A failure that stopped the review before it could apply changes.
#[derive(Debug, Error)]
pub enum SkillReviewError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Memory(#[from] MemoryError),
    #[error(transparent)]
    Skill(#[from] SkillError),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("review response is invalid: {0}")]
    Invalid(String),
}

/// Reviews the recent successful runs and updates the learned skill library.
pub struct SkillReviewer {
    session: Session,
    provider: Arc<dyn Provider>,
    pricing: Pricing,
    memory: Arc<MemoryService>,
    skills: Arc<SkillLibrary>,
    model: String,
    provider_name: String,
    max_tokens: u32,
}

impl SkillReviewer {
    /// Build a reviewer.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Session,
        provider: Arc<dyn Provider>,
        pricing: Pricing,
        memory: Arc<MemoryService>,
        skills: Arc<SkillLibrary>,
        model: String,
        provider_name: String,
        max_tokens: u32,
    ) -> Self {
        Self {
            session,
            provider,
            pricing,
            memory,
            skills,
            model,
            provider_name,
            max_tokens,
        }
    }

    /// Run one review and apply at most three model changes plus the automatic
    /// lifecycle transitions.
    pub fn run(&self, task_id: &str, now: i64) -> Result<ReviewReport, SkillReviewError> {
        let since = self.last_review(now)?;
        let submissions = self.session.submissions_settled_since(since)?;
        let mut success: HashMap<String, String> = HashMap::new();
        let mut texts: HashMap<String, String> = HashMap::new();
        for submission in &submissions {
            let done = submission.status == SubmissionStatus::Done;
            let history = self.session.history(&submission.conversation_id)?;
            for entry in history
                .iter()
                .filter(|entry| entry.created_at >= submission.created_at)
            {
                for name in skill_loads(entry) {
                    let _ = self.skills.record_outcome(&name, done)?;
                }
                if done && matches!(entry.kind.as_str(), "user" | "assistant") {
                    let text = entry_text(entry);
                    if !text.trim().is_empty() {
                        success.insert(entry.id.clone(), submission.id.clone());
                        texts.entry(entry.id.clone()).or_insert(text);
                    }
                }
            }
        }

        let mut report = ReviewReport {
            applied: self.apply_lifecycle(now)?,
            rejected: 0,
        };

        let (user_text, request) = self.build_prompt(&texts)?;
        let prompt_text = format!("{INSTRUCTION}\n\n{user_text}");
        self.session.save_prompt_blob(&prompt_text)?;
        let started = Instant::now();
        let stream = self
            .provider
            .stream(request)
            .map_err(|error| SkillReviewError::Provider(error.to_string()))?;
        let mut text = String::new();
        let mut usage = Usage::default();
        for event in stream {
            match event {
                Ok(StreamEvent::TextDelta(delta)) => text.push_str(&delta),
                Ok(StreamEvent::Usage(value)) => usage = value,
                Ok(_) => {}
                Err(error) => {
                    return Err(SkillReviewError::Provider(error.to_string()));
                }
            }
        }
        let latency_ms = started.elapsed().as_millis() as i64;
        self.record_usage(task_id, usage, latency_ms)?;
        let changes = parse_changes(&text)?;
        for change in changes.iter().take(MAX_CHANGES) {
            match self.apply_change(change, &success) {
                Ok(true) => report.applied += 1,
                Ok(false) => {}
                Err(error) => {
                    report.rejected += 1;
                    tracing::debug!(%error, "skill review change rejected");
                }
            }
        }
        Ok(report)
    }

    fn last_review(&self, now: i64) -> Result<i64, SkillReviewError> {
        if let Some(value) = self.session.meta_value("last_skill_review")?
            && let Ok(stamp) = OffsetDateTime::parse(&value, &Rfc3339)
        {
            return Ok(stamp.unix_timestamp_nanos().div_euclid(1_000) as i64);
        }
        Ok(now.saturating_sub(FIRST_REVIEW_DAYS * 86_400_000_000))
    }

    fn apply_lifecycle(&self, now: i64) -> Result<usize, SkillReviewError> {
        let mut applied = 0usize;
        for skill in self.skills.load_all()? {
            if skill.scope != SkillScope::Learned {
                continue;
            }
            let mut status = skill.status;
            if status == SkillStatus::Draft && skill.successes > 0 {
                self.skills.set_status(&skill.name, SkillStatus::Active)?;
                status = SkillStatus::Active;
                applied += 1;
            }
            if skill.failures >= 2
                && !matches!(status, SkillStatus::Quarantined | SkillStatus::Archived)
            {
                self.skills
                    .set_status(&skill.name, SkillStatus::Quarantined)?;
                status = SkillStatus::Quarantined;
                applied += 1;
            }
            let unused = unused_days(&skill, now);
            let bad_rate = skill.uses >= 4 && skill.success_rate() < 0.5;
            if matches!(status, SkillStatus::Active | SkillStatus::Draft)
                && (unused >= 90 || bad_rate)
            {
                self.skills.set_status(&skill.name, SkillStatus::Archived)?;
                applied += 1;
            }
        }
        Ok(applied)
    }

    fn build_prompt(
        &self,
        texts: &HashMap<String, String>,
    ) -> Result<(String, ChatRequest), SkillReviewError> {
        let mut recent = String::new();
        let mut ids: Vec<&String> = texts.keys().collect();
        ids.sort();
        for id in ids {
            let line = format!("[{id}] {}\n", texts[id]);
            if hearth_model::estimate_tokens(&format!("{recent}{line}")) > VIEW_TOKEN_CAP {
                continue;
            }
            recent.push_str(&line);
        }
        let mut skills = String::new();
        for skill in self.skills.load_all()? {
            if skill.status == SkillStatus::Archived {
                continue;
            }
            let line = format!(
                "## {} ({}, v{})\n{}\n{}\n",
                skill.name,
                skill.status.as_str(),
                skill.version,
                skill.description,
                skill.body
            );
            if hearth_model::estimate_tokens(&format!("{skills}{line}")) > SKILL_BODY_TOKEN_CAP {
                continue;
            }
            skills.push_str(&line);
        }
        let standing = self.memory.standing()?;
        let memory = self.memory.render_standing(&standing.items);
        let user_text = format!(
            "## Recent successful runs\n{recent}\n\n## Skills\n{skills}\n\n## Memory\n{memory}"
        );
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
            max_tokens: Some(self.max_tokens),
            session_id: Some("skill.review".to_owned()),
        };
        Ok((user_text, request))
    }

    fn apply_change(
        &self,
        change: &Value,
        success: &HashMap<String, String>,
    ) -> Result<bool, String> {
        let action = change
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| "change has no action".to_owned())?;
        if action == "noop" {
            return Ok(false);
        }
        let name = change
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "change has no name".to_owned())?;
        let evidence: Vec<String> = change
            .get("evidence")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if evidence.is_empty() {
            return Err("change has no evidence".to_owned());
        }
        for id in &evidence {
            if !success.contains_key(id) {
                return Err(format!("evidence is not from a successful run: {id}"));
            }
        }
        match action {
            "create" => {
                let submissions: BTreeSet<&str> = evidence
                    .iter()
                    .filter_map(|id| success.get(id).map(String::as_str))
                    .collect();
                if submissions.len() < 3 {
                    return Err("a new skill needs three successful runs".to_owned());
                }
                let description = change
                    .get("description")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "create needs a description".to_owned())?;
                let body = change
                    .get("body")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "create needs a body".to_owned())?;
                self.skills
                    .save_learned(SkillSave {
                        name: name.to_owned(),
                        description: description.to_owned(),
                        body: body.to_owned(),
                        evidence,
                    })
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            "revise" => {
                let existing = self
                    .skills
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("skill not found: {name}"))?;
                if existing.scope == SkillScope::User {
                    return Err(format!("skill is read-only: {name}"));
                }
                let description = change
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(&existing.description);
                let body = change
                    .get("body")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "revise needs a body".to_owned())?;
                self.skills
                    .save_learned(SkillSave {
                        name: name.to_owned(),
                        description: description.to_owned(),
                        body: body.to_owned(),
                        evidence,
                    })
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            "promote" => {
                let skill = self.learned(name)?;
                if skill.successes == 0 && skill.status != SkillStatus::Draft {
                    return Err("promote needs a successful use".to_owned());
                }
                self.skills
                    .set_status(name, SkillStatus::Active)
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            "quarantine" => {
                let skill = self.learned(name)?;
                if skill.failures < 2 {
                    return Err("quarantine needs two consecutive failures".to_owned());
                }
                self.skills
                    .set_status(name, SkillStatus::Quarantined)
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            "archive" => {
                self.learned(name)?;
                self.skills
                    .set_status(name, SkillStatus::Archived)
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            other => Err(format!("unknown skill change: {other}")),
        }
    }

    fn learned(&self, name: &str) -> Result<Skill, String> {
        let skill = self
            .skills
            .find(name)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("skill not found: {name}"))?;
        if skill.scope != SkillScope::Learned {
            return Err(format!("skill is read-only: {name}"));
        }
        Ok(skill)
    }

    fn record_usage(
        &self,
        task_id: &str,
        usage: Usage,
        latency_ms: i64,
    ) -> Result<(), SkillReviewError> {
        let cost_micros = self
            .pricing
            .price(&self.model)
            .map(|price| price.cost_micros(&usage))
            .unwrap_or(0);
        self.session.record_usage(NewUsage {
            task_id: Some(task_id.to_owned()),
            conversation_id: None,
            provider: self.provider_name.clone(),
            model: self.model.clone(),
            kind: "skill.review".to_owned(),
            usage,
            cost_micros,
            latency_ms,
        })?;
        Ok(())
    }
}

/// Runs the nightly skill review and sleeps until the next 04:30.
pub struct SkillReviewHandler {
    session: Session,
    reviewer: Arc<SkillReviewer>,
}

impl SkillReviewHandler {
    /// Build the handler.
    pub fn new(session: Session, reviewer: Arc<SkillReviewer>) -> Self {
        Self { session, reviewer }
    }
}

impl TaskHandler for SkillReviewHandler {
    fn step(
        &self,
        task: &hearth_core::ClaimedTask,
    ) -> Result<hearth_core::StepOutput, TaskHandlerError> {
        let now = self.session.now_micros();
        let next = next_daily_micros(now, local_offset(), SKILL_REVIEW_HOUR, SKILL_REVIEW_MINUTE);
        let report = match self.reviewer.run(&task.id, now) {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(%error, "skill review failed");
                ReviewReport::default()
            }
        };
        let stamp = rfc3339(now);
        self.session
            .set_meta("last_skill_review", &stamp)
            .map_err(|error| TaskHandlerError::Retryable(error.to_string()))?;
        self.session
            .set_meta("last_skill_review_changes", &report.applied.to_string())
            .map_err(|error| TaskHandlerError::Retryable(error.to_string()))?;
        Ok(hearth_core::StepOutput::Sleep {
            until_micros: next,
            phase: "run".to_owned(),
            checkpoint: json!({
                "last_skill_review": stamp,
                "applied": report.applied,
                "rejected": report.rejected,
            }),
        })
    }
}

fn skill_loads(entry: &Entry) -> Vec<String> {
    if entry.kind != "assistant" {
        return Vec::new();
    }
    parse_blocks(&entry.content)
        .into_iter()
        .filter_map(|block| match block {
            ContentBlock::ToolCall(call) if call.name == "skill" => {
                let action = call.arguments.get("action").and_then(Value::as_str)?;
                if action != "load" {
                    return None;
                }
                call.arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            _ => None,
        })
        .collect()
}

fn unused_days(skill: &Skill, now: i64) -> i64 {
    let stamp = skill
        .last_used
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(&skill.created);
    OffsetDateTime::parse(stamp, &Rfc3339)
        .map(|stamp| {
            (now - stamp.unix_timestamp_nanos().div_euclid(1_000) as i64).max(0) / 86_400_000_000
        })
        .unwrap_or(0)
}

fn parse_changes(text: &str) -> Result<Vec<Value>, SkillReviewError> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let body = body.strip_suffix("```").unwrap_or(body).trim();
    let value: Value =
        serde_json::from_str(body).map_err(|error| SkillReviewError::Invalid(error.to_string()))?;
    let changes = value
        .get("changes")
        .and_then(Value::as_array)
        .ok_or_else(|| SkillReviewError::Invalid("changes is not an array".to_owned()))?;
    Ok(changes.clone())
}
