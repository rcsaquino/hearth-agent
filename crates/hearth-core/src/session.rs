//! The harness, the single writer, and the save path.

use std::path::Path;
use std::sync::Arc;

use hearth_storage::{Storage, Writer};
use serde_json::Value;

use crate::clock::{Clock, IdGen, SystemClock, UuidV7};
use crate::error::CoreError;
use crate::model::{
    Commit, Conversation, Document, Entry, NewSubmission, Save, Submission, SubmissionStatus,
    SubmitResult,
};
use crate::task::{ClaimedTask, NewTask, RecoveryReport, StepOutput, Task, TaskStatus};

const CONVERSATION_SELECT: &str = "SELECT id, parent_id, parent_entry_id, title, transport, \
     transport_key, created_at, updated_at, archived FROM conversations";
const ENTRY_SELECT: &str = "SELECT id, conversation_id, seq, kind, content_json, tokens_est, \
     source, prompt_hash, created_at FROM entries";
const SUBMISSION_SELECT: &str = "SELECT id, conversation_id, request_id, kind, content_json, \
     status, answer_entry_id, error_json, created_at, settled_at FROM submissions";
const TASK_SELECT: &str = "SELECT id, conversation_id, parent_task_id, kind, version, status, \
     phase, attempt, input_json, checkpoint_json, result_json, lease_owner, lease_until, \
     next_wake_at, created_at, updated_at, terminal_at FROM tasks";

/// Open the permanent record and run work.
pub struct Harness {
    session: Session,
}

impl Harness {
    /// Open the database for a home folder.
    pub fn open(home: &Path) -> Result<Self, CoreError> {
        Self::open_with(home, Arc::new(SystemClock), Arc::new(UuidV7))
    }

    /// Open the database with an injected clock and identifier source.
    pub fn open_with(
        home: &Path,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGen>,
    ) -> Result<Self, CoreError> {
        let storage = Storage::open(&home.join("hearth.db"))?;
        Ok(Self {
            session: Session {
                storage: Arc::new(storage),
                clock,
                ids,
            },
        })
    }

    /// The single writer.
    pub fn session(&self) -> &Session {
        &self.session
    }
}

/// The one path every change goes through.
#[derive(Clone)]
pub struct Session {
    storage: Arc<Storage>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGen>,
}

impl Session {
    /// Create a conversation.
    pub fn create_conversation(
        &self,
        title: &str,
        transport: &str,
        transport_key: &str,
    ) -> Result<Conversation, CoreError> {
        let now = self.clock.now_micros();
        let conversation = Conversation {
            id: self.ids.new_id(),
            parent_id: None,
            parent_entry_id: None,
            title: title.to_owned(),
            transport: transport.to_owned(),
            transport_key: transport_key.to_owned(),
            created_at: now,
            updated_at: now,
            archived: false,
        };
        let insert = conversation.clone();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            writer.execute(
                "INSERT INTO conversations (id, parent_id, parent_entry_id, title, transport, \
                 transport_key, created_at, updated_at, archived) \
                 VALUES (?, NULL, NULL, ?, ?, ?, ?, ?, 0)",
                (
                    insert.id.as_str(),
                    insert.title.as_str(),
                    insert.transport.as_str(),
                    insert.transport_key.as_str(),
                    insert.created_at,
                    insert.updated_at,
                ),
            )?;
            Ok(())
        })?;
        Ok(conversation)
    }

    /// Perform one all-or-nothing save.
    pub fn save(&self, save: Save) -> Result<Commit, CoreError> {
        let now = self.clock.now_micros();
        let mut entry_ids = Vec::with_capacity(save.entries.len());
        for _ in &save.entries {
            entry_ids.push(self.ids.new_id());
        }
        let conversation_id = save.conversation_id.clone();

        self.storage.write(move |writer| -> Result<Commit, CoreError> {
            let exists: Option<i64> = writer.query_optional(
                "SELECT 1 FROM conversations WHERE id = ?",
                (conversation_id.as_str(),),
                |row| row.get(0),
            )?;
            if exists.is_none() {
                return Err(CoreError::ConversationNotFound(conversation_id.clone()));
            }

            for document in &save.documents {
                let found: Option<i64> = writer.query_optional(
                    "SELECT version FROM documents WHERE conversation_id = ? AND kind = ?",
                    (conversation_id.as_str(), document.kind.as_str()),
                    |row| row.get(0),
                )?;
                let expected = document.expected.map(|version| version as i64);
                if found != expected {
                    return Err(CoreError::DocumentVersion {
                        kind: document.kind.clone(),
                        expected: document.expected,
                        found: found.map(|version| version as u64),
                    });
                }
            }

            let seq: i64 = writer.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM commits",
                [],
                |row| row.get(0),
            )?;
            writer.execute(
                "INSERT INTO commits (seq, created_at, conversation_id, kind) \
                 VALUES (?, ?, ?, 'save')",
                (seq, now, conversation_id.as_str()),
            )?;

            for (entry, id) in save.entries.iter().zip(entry_ids.iter()) {
                let content = serde_json::to_string(&entry.content)
                    .map_err(|error| CoreError::Invalid(error.to_string()))?;
                writer.execute(
                    "INSERT INTO entries (id, conversation_id, seq, kind, content_json, \
                     tokens_est, source, prompt_hash, created_at) \
                     VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?)",
                    (
                        id.as_str(),
                        conversation_id.as_str(),
                        seq,
                        entry.kind.as_str(),
                        content.as_str(),
                        entry.source.as_deref(),
                        entry.prompt_hash.as_deref(),
                        now,
                    ),
                )?;
            }

            for document in &save.documents {
                let json = serde_json::to_string(&document.json)
                    .map_err(|error| CoreError::Invalid(error.to_string()))?;
                match document.expected {
                    None => {
                        writer.execute(
                            "INSERT INTO documents (conversation_id, kind, version, json, updated_at) \
                             VALUES (?, ?, 1, ?, ?)",
                            (
                                conversation_id.as_str(),
                                document.kind.as_str(),
                                json.as_str(),
                                now,
                            ),
                        )?;
                    }
                    Some(version) => {
                        let changed = writer.execute(
                            "UPDATE documents SET version = version + 1, json = ?, updated_at = ? \
                             WHERE conversation_id = ? AND kind = ? AND version = ?",
                            (
                                json.as_str(),
                                now,
                                conversation_id.as_str(),
                                document.kind.as_str(),
                                version as i64,
                            ),
                        )?;
                        if changed != 1 {
                            return Err(CoreError::DocumentVersion {
                                kind: document.kind.clone(),
                                expected: Some(version),
                                found: None,
                            });
                        }
                    }
                }
            }

            for submission in &save.submissions {
                let content = serde_json::to_string(&submission.content)
                    .map_err(|error| CoreError::Invalid(error.to_string()))?;
                writer.execute(
                    "INSERT INTO submissions (id, conversation_id, request_id, kind, content_json, \
                     status, answer_entry_id, error_json, created_at, settled_at) \
                     VALUES (?, ?, ?, ?, ?, 'queued', NULL, NULL, ?, NULL)",
                    (
                        submission.id.as_str(),
                        conversation_id.as_str(),
                        submission.request_id.as_str(),
                        submission.kind.as_str(),
                        content.as_str(),
                        now,
                    ),
                )?;
            }

            for settle in &save.settles {
                if let Some(answer_entry_id) = settle.answer_entry_id.as_deref() {
                    let found: Option<i64> = writer.query_optional(
                        "SELECT 1 FROM entries WHERE id = ? AND conversation_id = ?",
                        (answer_entry_id, conversation_id.as_str()),
                        |row| row.get(0),
                    )?;
                    if found.is_none() {
                        return Err(CoreError::EntryNotFound(
                            answer_entry_id.to_owned(),
                            conversation_id.clone(),
                        ));
                    }
                }
                let changed = writer.execute(
                    "UPDATE submissions SET status = ?, answer_entry_id = ?, settled_at = ? \
                     WHERE id = ? AND conversation_id = ? AND settled_at IS NULL",
                    (
                        settle.status.as_str(),
                        settle.answer_entry_id.as_deref(),
                        now,
                        settle.submission_id.as_str(),
                        conversation_id.as_str(),
                    ),
                )?;
                if changed != 1 {
                    let found: Option<(String, String)> = writer.query_optional(
                        "SELECT status, conversation_id FROM submissions WHERE id = ?",
                        (settle.submission_id.as_str(),),
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;
                    return match found {
                        Some((status, found_conversation)) => {
                            if found_conversation != conversation_id {
                                Err(CoreError::Invalid(format!(
                                    "submission {} belongs to another conversation",
                                    settle.submission_id
                                )))
                            } else {
                                Err(CoreError::AlreadySettled(
                                    settle.submission_id.clone(),
                                    status,
                                ))
                            }
                        }
                        None => Err(CoreError::SubmissionNotFound(
                            settle.submission_id.clone(),
                        )),
                    };
                }
            }

            writer.execute(
                "UPDATE conversations SET updated_at = ? WHERE id = ?",
                (now, conversation_id.as_str()),
            )?;

            Ok(Commit { seq, entry_ids })
        })
    }

    /// Hand in a message with a receipt.
    pub fn submit(
        &self,
        conversation_id: &str,
        request_id: &str,
        kind: &str,
        content: Value,
    ) -> Result<SubmitResult, CoreError> {
        if let Some(existing) = self.submission_by_request_id(request_id)? {
            return Ok(SubmitResult {
                submission: existing,
                created: false,
            });
        }

        let id = self.ids.new_id();
        let save = Save::new(conversation_id)
            .entry("user", content.clone())
            .submission(NewSubmission {
                id: id.clone(),
                request_id: request_id.to_owned(),
                kind: kind.to_owned(),
                content,
            });

        match self.save(save) {
            Ok(_) => {
                let submission = self
                    .submission_by_id(&id)?
                    .ok_or_else(|| CoreError::SubmissionNotFound(id.clone()))?;
                Ok(SubmitResult {
                    submission,
                    created: true,
                })
            }
            Err(CoreError::Storage(error)) if error.is_unique_violation() => {
                let submission = self
                    .submission_by_request_id(request_id)?
                    .ok_or_else(|| CoreError::SubmissionNotFound(request_id.to_owned()))?;
                Ok(SubmitResult {
                    submission,
                    created: false,
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Settle a submission exactly once.
    pub fn settle(
        &self,
        submission_id: &str,
        status: SubmissionStatus,
        answer_entry_id: Option<&str>,
    ) -> Result<Submission, CoreError> {
        if !matches!(
            status,
            SubmissionStatus::Done | SubmissionStatus::Unanswered | SubmissionStatus::Cancelled
        ) {
            return Err(CoreError::Invalid(format!(
                "{} is not a terminal submission status",
                status.as_str()
            )));
        }

        let existing = self
            .submission_by_id(submission_id)?
            .ok_or_else(|| CoreError::SubmissionNotFound(submission_id.to_owned()))?;
        if existing.settled_at.is_some() {
            return Err(CoreError::AlreadySettled(
                submission_id.to_owned(),
                existing.status.as_str().to_owned(),
            ));
        }

        let save = Save::new(&existing.conversation_id).settle(
            submission_id,
            status,
            answer_entry_id.map(str::to_owned),
        );
        self.save(save)?;
        self.submission_by_id(submission_id)?
            .ok_or_else(|| CoreError::SubmissionNotFound(submission_id.to_owned()))
    }

    /// Read one conversation.
    pub fn conversation(&self, id: &str) -> Result<Option<Conversation>, CoreError> {
        let sql = format!("{CONVERSATION_SELECT} WHERE id = ?");
        let raw: Option<RawConversation> = self.storage.read(|reader| {
            reader.query_optional(&sql, (id,), |row| {
                Ok(RawConversation {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    parent_entry_id: row.get(2)?,
                    title: row.get(3)?,
                    transport: row.get(4)?,
                    transport_key: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    archived: row.get(8)?,
                })
            })
        })?;
        Ok(raw.map(RawConversation::into_conversation))
    }

    /// Read the entries of a conversation in order.
    pub fn history(&self, conversation_id: &str) -> Result<Vec<Entry>, CoreError> {
        let sql = format!("{ENTRY_SELECT} WHERE conversation_id = ? ORDER BY seq, rowid");
        let raw: Vec<RawEntry> = self.storage.read(|reader| {
            reader.query(&sql, (conversation_id,), |row| {
                Ok(RawEntry {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    seq: row.get(2)?,
                    kind: row.get(3)?,
                    content_json: row.get(4)?,
                    tokens_est: row.get(5)?,
                    source: row.get(6)?,
                    prompt_hash: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
        })?;
        raw.into_iter().map(RawEntry::into_entry).collect()
    }

    /// Read one document.
    pub fn document(
        &self,
        conversation_id: &str,
        kind: &str,
    ) -> Result<Option<Document>, CoreError> {
        let raw: Option<(i64, String, i64)> = self.storage.read(|reader| {
            reader.query_optional(
                "SELECT version, json, updated_at FROM documents \
                 WHERE conversation_id = ? AND kind = ?",
                (conversation_id, kind),
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })?;
        match raw {
            Some((version, json, updated_at)) => Ok(Some(Document {
                conversation_id: conversation_id.to_owned(),
                kind: kind.to_owned(),
                version: version as u64,
                json: serde_json::from_str(&json)
                    .map_err(|error| CoreError::Corrupt(error.to_string()))?,
                updated_at,
            })),
            None => Ok(None),
        }
    }

    /// Read one submission by its request id.
    pub fn submission_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<Submission>, CoreError> {
        self.read_submission("request_id", request_id)
    }

    /// Read one submission by its id.
    pub fn submission_by_id(&self, id: &str) -> Result<Option<Submission>, CoreError> {
        self.read_submission("id", id)
    }

    /// Create a task in its first phase.
    pub fn create_task(&self, new: NewTask) -> Result<Task, CoreError> {
        if new.kind.trim().is_empty() {
            return Err(CoreError::Invalid("task kind is empty".to_owned()));
        }
        if new.phase.trim().is_empty() {
            return Err(CoreError::Invalid("task phase is empty".to_owned()));
        }
        if new.version < 1 {
            return Err(CoreError::Invalid(
                "task version must be positive".to_owned(),
            ));
        }

        let now = self.clock.now_micros();
        let id = self.ids.new_id();
        let input = serde_json::to_string(&new.input)
            .map_err(|error| CoreError::Invalid(error.to_string()))?;
        let task = Task {
            id: id.clone(),
            conversation_id: new.conversation_id,
            parent_task_id: new.parent_task_id,
            kind: new.kind,
            version: new.version,
            status: TaskStatus::Pending,
            phase: new.phase,
            attempt: 0,
            input: new.input,
            checkpoint: None,
            result: None,
            lease_owner: None,
            lease_until: None,
            next_wake_at: None,
            created_at: now,
            updated_at: now,
            terminal_at: None,
        };
        let insert = task.clone();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            writer.execute(
                "INSERT INTO tasks (id, conversation_id, parent_task_id, kind, version, status, \
                 phase, attempt, input_json, checkpoint_json, result_json, lease_owner, \
                 lease_until, next_wake_at, created_at, updated_at, terminal_at) \
                 VALUES (?, ?, ?, ?, ?, 'pending', ?, 0, ?, NULL, NULL, NULL, NULL, NULL, ?, ?, NULL)",
                (
                    insert.id.as_str(),
                    insert.conversation_id.as_deref(),
                    insert.parent_task_id.as_deref(),
                    insert.kind.as_str(),
                    insert.version,
                    insert.phase.as_str(),
                    input.as_str(),
                    insert.created_at,
                    insert.updated_at,
                ),
            )?;
            Ok(())
        })?;
        Ok(task)
    }

    /// Read one task.
    pub fn task(&self, id: &str) -> Result<Option<Task>, CoreError> {
        let sql = format!("{TASK_SELECT} WHERE id = ?");
        let raw: Option<RawTask> = self.storage.read(|reader| {
            reader.query_optional(&sql, (id,), |row| {
                Ok(RawTask {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    parent_task_id: row.get(2)?,
                    kind: row.get(3)?,
                    version: row.get(4)?,
                    status: row.get(5)?,
                    phase: row.get(6)?,
                    attempt: row.get(7)?,
                    input_json: row.get(8)?,
                    checkpoint_json: row.get(9)?,
                    result_json: row.get(10)?,
                    lease_owner: row.get(11)?,
                    lease_until: row.get(12)?,
                    next_wake_at: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                    terminal_at: row.get(16)?,
                })
            })
        })?;
        raw.map(RawTask::into_task).transpose()
    }

    /// Claim the oldest runnable task.
    pub fn claim_task(
        &self,
        owner: &str,
        lease_micros: i64,
    ) -> Result<Option<ClaimedTask>, CoreError> {
        if owner.trim().is_empty() {
            return Err(CoreError::Invalid("task owner is empty".to_owned()));
        }
        if lease_micros <= 0 {
            return Err(CoreError::Invalid("task lease must be positive".to_owned()));
        }
        let now = self.clock.now_micros();
        let owner = owner.to_owned();
        self.storage
            .write(move |writer| -> Result<Option<ClaimedTask>, CoreError> {
                let row: Option<RawClaim> = writer.query_optional(
                    "SELECT id, kind, phase, attempt, input_json, checkpoint_json FROM tasks                      WHERE status = 'pending' AND (next_wake_at IS NULL OR next_wake_at <= ?)                      ORDER BY COALESCE(next_wake_at, created_at), id LIMIT 1",
                    (now,),
                    |row| {
                        Ok(RawClaim {
                            id: row.get(0)?,
                            kind: row.get(1)?,
                            phase: row.get(2)?,
                            attempt: row.get(3)?,
                            input_json: row.get(4)?,
                            checkpoint_json: row.get(5)?,
                        })
                    },
                )?;
                let Some(row) = row else {
                    return Ok(None);
                };
                let attempt = row.attempt + 1;
                let changed = writer.execute(
                    "UPDATE tasks SET status = 'running', lease_owner = ?, lease_until = ?, \
                     attempt = ?, updated_at = ? WHERE id = ? AND status = 'pending'",
                    (
                        owner.as_str(),
                        now + lease_micros,
                        attempt,
                        now,
                        row.id.as_str(),
                    ),
                )?;
                if changed != 1 {
                    return Ok(None);
                }
                let input = serde_json::from_str(&row.input_json)
                    .map_err(|error| CoreError::Corrupt(error.to_string()))?;
                let checkpoint = row
                    .checkpoint_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|error| CoreError::Corrupt(error.to_string()))?;
                Ok(Some(ClaimedTask {
                    id: row.id,
                    kind: row.kind,
                    phase: row.phase,
                    attempt,
                    input,
                    checkpoint,
                }))
            })
    }

    /// Extend the lease on a running task.
    pub fn heartbeat_task(
        &self,
        task_id: &str,
        _owner: &str,
        _lease_micros: i64,
    ) -> Result<(), CoreError> {
        Err(CoreError::TaskNotRunning(
            task_id.to_owned(),
            "not implemented".to_owned(),
        ))
    }

    /// Apply one step outcome.
    pub fn finish_task(
        &self,
        task_id: &str,
        owner: &str,
        output: StepOutput,
    ) -> Result<Task, CoreError> {
        let now = self.clock.now_micros();
        let task_id = task_id.to_owned();
        let owner = owner.to_owned();
        self.storage
            .write(move |writer| -> Result<Task, CoreError> {
                let task = load_task(writer, &task_id)?
                    .ok_or_else(|| CoreError::TaskNotFound(task_id.clone()))?;
                if task.status != TaskStatus::Running {
                    return Err(CoreError::TaskNotRunning(
                        task_id.clone(),
                        task.status.as_str().to_owned(),
                    ));
                }
                if task.lease_owner.as_deref() != Some(owner.as_str()) {
                    return Err(CoreError::LeaseLost(task_id.clone()));
                }

                match output {
                    StepOutput::Next { phase, checkpoint } => {
                        let checkpoint = serde_json::to_string(&checkpoint)
                            .map_err(|error| CoreError::Invalid(error.to_string()))?;
                        let changed = writer.execute(
                            "UPDATE tasks SET status = 'pending', phase = ?, checkpoint_json = ?, \
                         attempt = 0, lease_owner = NULL, lease_until = NULL, next_wake_at = NULL, \
                         updated_at = ? WHERE id = ? AND status = 'running' AND lease_owner = ?",
                            (phase, checkpoint, now, task_id.as_str(), owner.as_str()),
                        )?;
                        if changed != 1 {
                            return Err(CoreError::LeaseLost(task_id));
                        }
                    }
                    _ => {
                        return Err(CoreError::Invalid(
                            "step outcome is not implemented".to_owned(),
                        ));
                    }
                }

                load_task(writer, &task_id)?.ok_or(CoreError::TaskNotFound(task_id))
            })
    }

    /// Requeue tasks whose leases or sleeps expired.
    pub fn recover_tasks(&self) -> Result<RecoveryReport, CoreError> {
        Ok(RecoveryReport::default())
    }

    /// Fork a conversation at a chosen entry.
    pub fn fork(&self, conversation_id: &str, entry_id: &str) -> Result<Conversation, CoreError> {
        let now = self.clock.now_micros();
        let ids = Arc::clone(&self.ids);
        let source_id = conversation_id.to_owned();
        let chosen = entry_id.to_owned();

        self.storage
            .write(move |writer| -> Result<Conversation, CoreError> {
                let conversation_sql = format!("{CONVERSATION_SELECT} WHERE id = ?");
                let source: Option<RawConversation> =
                    writer.query_optional(&conversation_sql, (source_id.as_str(),), |row| {
                        Ok(RawConversation {
                            id: row.get(0)?,
                            parent_id: row.get(1)?,
                            parent_entry_id: row.get(2)?,
                            title: row.get(3)?,
                            transport: row.get(4)?,
                            transport_key: row.get(5)?,
                            created_at: row.get(6)?,
                            updated_at: row.get(7)?,
                            archived: row.get(8)?,
                        })
                    })?;
                let source =
                    source.ok_or_else(|| CoreError::ConversationNotFound(source_id.clone()))?;

                let entry_sql =
                    format!("{ENTRY_SELECT} WHERE conversation_id = ? ORDER BY seq, rowid");
                let rows: Vec<RawEntry> =
                    writer.query(&entry_sql, (source_id.as_str(),), |row| {
                        Ok(RawEntry {
                            id: row.get(0)?,
                            conversation_id: row.get(1)?,
                            seq: row.get(2)?,
                            kind: row.get(3)?,
                            content_json: row.get(4)?,
                            tokens_est: row.get(5)?,
                            source: row.get(6)?,
                            prompt_hash: row.get(7)?,
                            created_at: row.get(8)?,
                        })
                    })?;
                let mut entries = Vec::with_capacity(rows.len());
                for row in rows {
                    entries.push(row.into_entry()?);
                }
                let cut = entries
                    .iter()
                    .position(|entry| entry.id == chosen)
                    .ok_or_else(|| CoreError::EntryNotFound(chosen.clone(), source_id.clone()))?;
                let copied: Vec<Entry> = entries.into_iter().take(cut + 1).collect();

                let fork = Conversation {
                    id: ids.new_id(),
                    parent_id: Some(source.id.clone()),
                    parent_entry_id: Some(chosen.clone()),
                    title: source.title.clone(),
                    transport: source.transport.clone(),
                    transport_key: source.transport_key.clone(),
                    created_at: now,
                    updated_at: now,
                    archived: false,
                };

                writer.execute(
                    "INSERT INTO conversations (id, parent_id, parent_entry_id, title, transport, \
                 transport_key, created_at, updated_at, archived) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)",
                    (
                        fork.id.as_str(),
                        fork.parent_id.as_deref(),
                        fork.parent_entry_id.as_deref(),
                        fork.title.as_str(),
                        fork.transport.as_str(),
                        fork.transport_key.as_str(),
                        fork.created_at,
                        fork.updated_at,
                    ),
                )?;

                let seq: i64 = writer.query_row(
                    "SELECT COALESCE(MAX(seq), 0) + 1 FROM commits",
                    [],
                    |row| row.get(0),
                )?;
                writer.execute(
                    "INSERT INTO commits (seq, created_at, conversation_id, kind) \
                 VALUES (?, ?, ?, 'fork')",
                    (seq, now, fork.id.as_str()),
                )?;

                for entry in &copied {
                    let id = ids.new_id();
                    let content = serde_json::to_string(&entry.content)
                        .map_err(|error| CoreError::Invalid(error.to_string()))?;
                    writer.execute(
                        "INSERT INTO entries (id, conversation_id, seq, kind, content_json, \
                     tokens_est, source, prompt_hash, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        (
                            id.as_str(),
                            fork.id.as_str(),
                            seq,
                            entry.kind.as_str(),
                            content.as_str(),
                            entry.tokens_est,
                            entry.source.as_deref(),
                            entry.prompt_hash.as_deref(),
                            entry.created_at,
                        ),
                    )?;
                }

                Ok(fork)
            })
    }

    fn read_submission(&self, column: &str, value: &str) -> Result<Option<Submission>, CoreError> {
        let sql = format!("{SUBMISSION_SELECT} WHERE {column} = ?");
        let raw: Option<RawSubmission> = self.storage.read(|reader| {
            reader.query_optional(&sql, (value,), |row| {
                Ok(RawSubmission {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    request_id: row.get(2)?,
                    kind: row.get(3)?,
                    content_json: row.get(4)?,
                    status: row.get(5)?,
                    answer_entry_id: row.get(6)?,
                    error_json: row.get(7)?,
                    created_at: row.get(8)?,
                    settled_at: row.get(9)?,
                })
            })
        })?;
        raw.map(RawSubmission::into_submission).transpose()
    }
}

struct RawConversation {
    id: String,
    parent_id: Option<String>,
    parent_entry_id: Option<String>,
    title: String,
    transport: String,
    transport_key: String,
    created_at: i64,
    updated_at: i64,
    archived: i64,
}

impl RawConversation {
    fn into_conversation(self) -> Conversation {
        Conversation {
            id: self.id,
            parent_id: self.parent_id,
            parent_entry_id: self.parent_entry_id,
            title: self.title,
            transport: self.transport,
            transport_key: self.transport_key,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived: self.archived != 0,
        }
    }
}

struct RawEntry {
    id: String,
    conversation_id: String,
    seq: i64,
    kind: String,
    content_json: String,
    tokens_est: i64,
    source: Option<String>,
    prompt_hash: Option<String>,
    created_at: i64,
}

impl RawEntry {
    fn into_entry(self) -> Result<Entry, CoreError> {
        let content = serde_json::from_str(&self.content_json)
            .map_err(|error| CoreError::Corrupt(error.to_string()))?;
        Ok(Entry {
            id: self.id,
            conversation_id: self.conversation_id,
            seq: self.seq,
            kind: self.kind,
            content,
            tokens_est: self.tokens_est,
            source: self.source,
            prompt_hash: self.prompt_hash,
            created_at: self.created_at,
        })
    }
}

struct RawSubmission {
    id: String,
    conversation_id: String,
    request_id: String,
    kind: String,
    content_json: String,
    status: String,
    answer_entry_id: Option<String>,
    error_json: Option<String>,
    created_at: i64,
    settled_at: Option<i64>,
}

impl RawSubmission {
    fn into_submission(self) -> Result<Submission, CoreError> {
        let content = serde_json::from_str(&self.content_json)
            .map_err(|error| CoreError::Corrupt(error.to_string()))?;
        let status = SubmissionStatus::parse(&self.status).ok_or_else(|| {
            CoreError::Corrupt(format!("unknown submission status: {}", self.status))
        })?;
        let error = match self.error_json {
            Some(json) => Some(
                serde_json::from_str(&json)
                    .map_err(|error| CoreError::Corrupt(error.to_string()))?,
            ),
            None => None,
        };
        Ok(Submission {
            id: self.id,
            conversation_id: self.conversation_id,
            request_id: self.request_id,
            kind: self.kind,
            content,
            status,
            answer_entry_id: self.answer_entry_id,
            error,
            created_at: self.created_at,
            settled_at: self.settled_at,
        })
    }
}

struct RawTask {
    id: String,
    conversation_id: Option<String>,
    parent_task_id: Option<String>,
    kind: String,
    version: i64,
    status: String,
    phase: String,
    attempt: i64,
    input_json: String,
    checkpoint_json: Option<String>,
    result_json: Option<String>,
    lease_owner: Option<String>,
    lease_until: Option<i64>,
    next_wake_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    terminal_at: Option<i64>,
}

impl RawTask {
    fn into_task(self) -> Result<Task, CoreError> {
        let status = TaskStatus::parse(&self.status)
            .ok_or_else(|| CoreError::Corrupt(format!("unknown task status: {}", self.status)))?;
        let input = serde_json::from_str(&self.input_json)
            .map_err(|error| CoreError::Corrupt(error.to_string()))?;
        let checkpoint = self
            .checkpoint_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|error| CoreError::Corrupt(error.to_string()))?;
        let result = self
            .result_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|error| CoreError::Corrupt(error.to_string()))?;
        Ok(Task {
            id: self.id,
            conversation_id: self.conversation_id,
            parent_task_id: self.parent_task_id,
            kind: self.kind,
            version: self.version,
            status,
            phase: self.phase,
            attempt: self.attempt,
            input,
            checkpoint,
            result,
            lease_owner: self.lease_owner,
            lease_until: self.lease_until,
            next_wake_at: self.next_wake_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            terminal_at: self.terminal_at,
        })
    }
}

struct RawClaim {
    id: String,
    kind: String,
    phase: String,
    attempt: i64,
    input_json: String,
    checkpoint_json: Option<String>,
}

fn load_task(writer: &Writer<'_>, id: &str) -> Result<Option<Task>, CoreError> {
    let sql = format!("{TASK_SELECT} WHERE id = ?");
    let raw: Option<RawTask> = writer.query_optional(&sql, (id,), |row| {
        Ok(RawTask {
            id: row.get(0)?,
            conversation_id: row.get(1)?,
            parent_task_id: row.get(2)?,
            kind: row.get(3)?,
            version: row.get(4)?,
            status: row.get(5)?,
            phase: row.get(6)?,
            attempt: row.get(7)?,
            input_json: row.get(8)?,
            checkpoint_json: row.get(9)?,
            result_json: row.get(10)?,
            lease_owner: row.get(11)?,
            lease_until: row.get(12)?,
            next_wake_at: row.get(13)?,
            created_at: row.get(14)?,
            updated_at: row.get(15)?,
            terminal_at: row.get(16)?,
        })
    })?;
    raw.map(RawTask::into_task).transpose()
}
