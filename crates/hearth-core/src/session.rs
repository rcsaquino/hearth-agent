//! The harness, the single writer, and the save path.

use std::path::Path;
use std::sync::Arc;

use hearth_storage::{Storage, Writer};
use serde_json::Value;
use uuid::Uuid;

use crate::clock::{Clock, IdGen, SystemClock, UuidV7};
use crate::error::CoreError;
use crate::model::{
    Commit, CommitRecord, Conversation, Document, Effect, EffectStatus, Entry, MemoryChange,
    MemoryUpdate, MemoryWrite, NewEntry, NewMemory, NewSubmission, NewUsage, PromptBlob, Save,
    Submission, SubmissionStatus, SubmitResult, TaskCounts, TelegramAdmission, TelegramUpdate,
    UsageEvent, UsageTotals,
};
use crate::prompt::{Prompt, PromptInput};
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
const EFFECT_SELECT: &str = "SELECT id, task_id, kind, key, payload_json, status, result_json, \
     attempts, created_at, settled_at FROM effects";

type RawCommitRow = (i64, i64, Option<String>, Option<String>, String);

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
    /// The durable storage handle used by the memory subsystem for reads.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// A new time-ordered identifier from the configured source.
    pub fn new_id(&self) -> String {
        self.ids.new_id()
    }

    /// The current time in UTC microseconds from the configured clock.
    pub fn now_micros(&self) -> i64 {
        self.clock.now_micros()
    }

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
        let mut effect_ids = Vec::with_capacity(save.effects.len());
        for _ in &save.effects {
            effect_ids.push(self.ids.new_id());
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

            let seq = next_sequence(writer)?;
            for (entry, id) in save.entries.iter().zip(entry_ids.iter()) {
                let content = serde_json::to_string(&entry.content)
                    .map_err(|error| CoreError::Invalid(error.to_string()))?;
                let tokens_est = crate::prompt::estimate_content(&entry.content);
                writer.execute(
                    "INSERT INTO entries (id, conversation_id, seq, kind, content_json, \
                     tokens_est, source, prompt_hash, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        id.as_str(),
                        conversation_id.as_str(),
                        seq,
                        entry.kind.as_str(),
                        content.as_str(),
                        tokens_est,
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
                let error_json = settle
                    .error
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| CoreError::Invalid(error.to_string()))?;
                let changed = writer.execute(
                    "UPDATE submissions SET status = ?, answer_entry_id = ?, error_json = ?, \
                     settled_at = ? WHERE id = ? AND conversation_id = ? AND settled_at IS NULL",
                    (
                        settle.status.as_str(),
                        settle.answer_entry_id.as_deref(),
                        error_json.as_deref(),
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

            let mut recorded_effect_ids = Vec::with_capacity(save.effects.len());
            apply_memory(writer, &save.memory, now)?;
            for (intent, generated_id) in save.effects.iter().zip(effect_ids.iter()) {
                let existing: Option<String> = writer.query_optional(
                    "SELECT id FROM effects WHERE kind = ? AND key = ?",
                    (intent.kind.as_str(), intent.key.as_str()),
                    |row| row.get(0),
                )?;
                let id = match existing {
                    Some(id) => id,
                    None => {
                        let payload = serde_json::to_string(&intent.payload)
                            .map_err(|error| CoreError::Invalid(error.to_string()))?;
                        writer.execute(
                            "INSERT INTO effects (id, task_id, kind, key, payload_json, status, \
                             result_json, attempts, created_at, settled_at) \
                             VALUES (?, ?, ?, ?, ?, 'intent', NULL, 0, ?, NULL)",
                            (
                                generated_id.as_str(),
                                intent.task_id.as_deref(),
                                intent.kind.as_str(),
                                intent.key.as_str(),
                                payload.as_str(),
                                now,
                            ),
                        )?;
                        generated_id.clone()
                    }
                };
                recorded_effect_ids.push(id);
            }

            if let Some(update) = &save.telegram_update {
                writer.execute(
                    "INSERT INTO telegram_updates (update_id, chat_id, message_id, processed_at) \
                     VALUES (?, ?, ?, ?)",
                    (update.update_id, update.chat_id, update.message_id, now),
                )?;
            }
            if let Some(offset) = save.telegram_offset {
                writer.execute(
                    "INSERT INTO meta (key, value) VALUES ('telegram_offset', ?) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    (offset.to_string(),),
                )?;
            }

            writer.execute(
                "UPDATE conversations SET updated_at = ? WHERE id = ?",
                (now, conversation_id.as_str()),
            )?;

            insert_commit(
                writer,
                seq,
                now,
                Some(conversation_id.as_str()),
                None,
                "save",
            )?;

            Ok(Commit {
                seq,
                entry_ids,
                effect_ids: recorded_effect_ids,
            })
        })
    }

    /// Read the newest line in the save log.
    pub fn latest_commit(&self) -> Result<Option<CommitRecord>, CoreError> {
        let raw: Option<RawCommitRow> = self.storage.read(|reader| {
            reader.query_optional(
                "SELECT seq, created_at, conversation_id, task_id, kind FROM commits \
                     ORDER BY seq DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
        })?;
        Ok(raw.map(
            |(seq, created_at, conversation_id, task_id, kind)| CommitRecord {
                seq,
                created_at,
                conversation_id,
                task_id,
                kind,
            },
        ))
    }

    /// Read one recorded effect.
    pub fn effect(&self, id: &str) -> Result<Option<Effect>, CoreError> {
        let sql = format!("{EFFECT_SELECT} WHERE id = ?");
        let raw: Option<RawEffect> = self.storage.read(|reader| {
            reader.query_optional(&sql, (id,), |row| {
                Ok(RawEffect {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    kind: row.get(2)?,
                    key: row.get(3)?,
                    payload_json: row.get(4)?,
                    status: row.get(5)?,
                    result_json: row.get(6)?,
                    attempts: row.get(7)?,
                    created_at: row.get(8)?,
                    settled_at: row.get(9)?,
                })
            })
        })?;
        raw.map(RawEffect::into_effect).transpose()
    }

    /// List effect intents that are ready to run, oldest first.
    pub fn pending_effects(&self, limit: usize) -> Result<Vec<Effect>, CoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        self.effects_with_status("intent", limit.min(100) as i64)
    }

    /// List effects that were started but not settled.
    pub fn started_effects(&self) -> Result<Vec<Effect>, CoreError> {
        self.effects_with_status("started", 100)
    }

    /// Read one effect by its kind and stable key.
    pub fn effect_by_key(&self, kind: &str, key: &str) -> Result<Option<Effect>, CoreError> {
        let sql =
            format!("{EFFECT_SELECT} WHERE kind = ? AND key = ? ORDER BY created_at DESC LIMIT 1");
        let raw: Option<RawEffect> = self.storage.read(|reader| {
            reader.query_optional(&sql, (kind, key), |row| {
                Ok(RawEffect {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    kind: row.get(2)?,
                    key: row.get(3)?,
                    payload_json: row.get(4)?,
                    status: row.get(5)?,
                    result_json: row.get(6)?,
                    attempts: row.get(7)?,
                    created_at: row.get(8)?,
                    settled_at: row.get(9)?,
                })
            })
        })?;
        raw.map(RawEffect::into_effect).transpose()
    }

    /// List effects awaiting delivery of one kind: intent or unknown, oldest first.
    pub fn effects_awaiting_delivery(&self, kind: &str) -> Result<Vec<Effect>, CoreError> {
        let sql = format!(
            "{EFFECT_SELECT} WHERE kind = ? AND status IN ('intent','unknown') \
             ORDER BY created_at, id"
        );
        let raw: Vec<RawEffect> = self.storage.read(|reader| {
            reader.query(&sql, (kind,), |row| {
                Ok(RawEffect {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    kind: row.get(2)?,
                    key: row.get(3)?,
                    payload_json: row.get(4)?,
                    status: row.get(5)?,
                    result_json: row.get(6)?,
                    attempts: row.get(7)?,
                    created_at: row.get(8)?,
                    settled_at: row.get(9)?,
                })
            })
        })?;
        raw.into_iter().map(RawEffect::into_effect).collect()
    }

    fn effects_with_status(&self, status: &str, limit: i64) -> Result<Vec<Effect>, CoreError> {
        let sql = format!("{EFFECT_SELECT} WHERE status = ? ORDER BY created_at, id LIMIT ?");
        let raw: Vec<RawEffect> = self.storage.read(|reader| {
            reader.query(&sql, (status, limit), |row| {
                Ok(RawEffect {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    kind: row.get(2)?,
                    key: row.get(3)?,
                    payload_json: row.get(4)?,
                    status: row.get(5)?,
                    result_json: row.get(6)?,
                    attempts: row.get(7)?,
                    created_at: row.get(8)?,
                    settled_at: row.get(9)?,
                })
            })
        })?;
        raw.into_iter().map(RawEffect::into_effect).collect()
    }

    /// Mark an effect intent as started.
    pub fn begin_effect(&self, id: &str) -> Result<Effect, CoreError> {
        let now = self.clock.now_micros();
        let effect_id = id.to_owned();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            let effect = load_effect(writer, &effect_id)?
                .ok_or_else(|| CoreError::EffectNotFound(effect_id.clone()))?;
            if effect.status != EffectStatus::Intent {
                return Err(CoreError::EffectNotReady {
                    id: effect_id,
                    status: effect.status.as_str().to_owned(),
                });
            }
            writer.execute(
                "UPDATE effects SET status = 'started', attempts = attempts + 1, \
                     settled_at = NULL WHERE id = ?",
                (effect_id.as_str(),),
            )?;
            record_commit(
                writer,
                now,
                None,
                effect.task_id.as_deref(),
                "effect.started",
            )?;
            Ok(())
        })?;
        self.effect(id)?
            .ok_or_else(|| CoreError::EffectNotFound(id.to_owned()))
    }

    /// Settle a started effect.
    pub fn finish_effect(&self, id: &str, result: Value) -> Result<Effect, CoreError> {
        let now = self.clock.now_micros();
        let effect_id = id.to_owned();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            let effect = load_effect(writer, &effect_id)?
                .ok_or_else(|| CoreError::EffectNotFound(effect_id.clone()))?;
            if effect.status != EffectStatus::Started {
                return Err(CoreError::EffectNotReady {
                    id: effect_id,
                    status: effect.status.as_str().to_owned(),
                });
            }
            let result_json = serde_json::to_string(&result)
                .map_err(|error| CoreError::Invalid(error.to_string()))?;
            writer.execute(
                "UPDATE effects SET status = 'done', result_json = ?, settled_at = ? \
                     WHERE id = ?",
                (result_json.as_str(), now, effect_id.as_str()),
            )?;
            record_commit(writer, now, None, effect.task_id.as_deref(), "effect.done")?;
            Ok(())
        })?;
        self.effect(id)?
            .ok_or_else(|| CoreError::EffectNotFound(id.to_owned()))
    }

    /// Record an effect failure, retryable or terminal.
    pub fn fail_effect(
        &self,
        id: &str,
        error: Value,
        retryable: bool,
    ) -> Result<Effect, CoreError> {
        let now = self.clock.now_micros();
        let effect_id = id.to_owned();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            let effect = load_effect(writer, &effect_id)?
                .ok_or_else(|| CoreError::EffectNotFound(effect_id.clone()))?;
            if effect.status != EffectStatus::Started {
                return Err(CoreError::EffectNotReady {
                    id: effect_id,
                    status: effect.status.as_str().to_owned(),
                });
            }
            let error_json = serde_json::to_string(&error)
                .map_err(|error| CoreError::Invalid(error.to_string()))?;
            let kind = if retryable {
                writer.execute(
                    "UPDATE effects SET status = 'intent', result_json = ?, \
                         settled_at = NULL WHERE id = ?",
                    (error_json.as_str(), effect_id.as_str()),
                )?;
                "effect.retry"
            } else {
                writer.execute(
                    "UPDATE effects SET status = 'failed', result_json = ?, settled_at = ? \
                         WHERE id = ?",
                    (error_json.as_str(), now, effect_id.as_str()),
                )?;
                "effect.failed"
            };
            record_commit(writer, now, None, effect.task_id.as_deref(), kind)?;
            Ok(())
        })?;
        self.effect(id)?
            .ok_or_else(|| CoreError::EffectNotFound(id.to_owned()))
    }

    /// Return a started effect to the intent state after a crash.
    pub fn requeue_effect(&self, id: &str) -> Result<Effect, CoreError> {
        let now = self.clock.now_micros();
        let effect_id = id.to_owned();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            let effect = load_effect(writer, &effect_id)?
                .ok_or_else(|| CoreError::EffectNotFound(effect_id.clone()))?;
            if effect.status != EffectStatus::Started {
                return Err(CoreError::EffectNotReady {
                    id: effect_id,
                    status: effect.status.as_str().to_owned(),
                });
            }
            writer.execute(
                "UPDATE effects SET status = 'intent', settled_at = NULL WHERE id = ?",
                (effect_id.as_str(),),
            )?;
            record_commit(
                writer,
                now,
                None,
                effect.task_id.as_deref(),
                "effect.requeued",
            )?;
            Ok(())
        })?;
        self.effect(id)?
            .ok_or_else(|| CoreError::EffectNotFound(id.to_owned()))
    }

    /// Mark an effect done, whether it was an intent, started, or unknown.
    pub fn mark_effect_done(&self, id: &str, result: Value) -> Result<Effect, CoreError> {
        self.set_effect_status(id, EffectStatus::Done, result, "effect.done")
    }

    /// Mark an effect failed, whether it was an intent, started, or unknown.
    pub fn mark_effect_failed(&self, id: &str, error: Value) -> Result<Effect, CoreError> {
        self.set_effect_status(id, EffectStatus::Failed, error, "effect.failed")
    }

    /// Mark an effect unknown, keeping its payload for a later recovery.
    pub fn mark_effect_unknown(&self, id: &str, result: Value) -> Result<Effect, CoreError> {
        let effect = self
            .effect(id)?
            .ok_or_else(|| CoreError::EffectNotFound(id.to_owned()))?;
        if effect.status == EffectStatus::Unknown {
            return Ok(effect);
        }
        self.set_effect_status(id, EffectStatus::Unknown, result, "effect.unknown")
    }

    fn set_effect_status(
        &self,
        id: &str,
        status: EffectStatus,
        result: Value,
        commit_kind: &str,
    ) -> Result<Effect, CoreError> {
        let now = self.clock.now_micros();
        let effect_id = id.to_owned();
        let commit_kind = commit_kind.to_owned();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            let effect = load_effect(writer, &effect_id)?
                .ok_or_else(|| CoreError::EffectNotFound(effect_id.clone()))?;
            if matches!(effect.status, EffectStatus::Done | EffectStatus::Failed) {
                return Err(CoreError::EffectNotReady {
                    id: effect_id,
                    status: effect.status.as_str().to_owned(),
                });
            }
            let result_json = serde_json::to_string(&result)
                .map_err(|error| CoreError::Invalid(error.to_string()))?;
            writer.execute(
                "UPDATE effects SET status = ?, result_json = ?, settled_at = ? WHERE id = ?",
                (
                    status.as_str(),
                    result_json.as_str(),
                    now,
                    effect_id.as_str(),
                ),
            )?;
            record_commit(writer, now, None, effect.task_id.as_deref(), &commit_kind)?;
            Ok(())
        })?;
        self.effect(id)?
            .ok_or_else(|| CoreError::EffectNotFound(id.to_owned()))
    }

    /// Write memory changes without a conversation. The save still has one commit.
    pub fn save_memory(&self, writes: Vec<MemoryWrite>) -> Result<Commit, CoreError> {
        let now = self.clock.now_micros();
        self.storage
            .write(move |writer| -> Result<Commit, CoreError> {
                let seq = next_sequence(writer)?;
                apply_memory(writer, &writes, now)?;
                insert_commit(writer, seq, now, None, None, "memory")?;
                Ok(Commit {
                    seq,
                    entry_ids: Vec::new(),
                    effect_ids: Vec::new(),
                })
            })
    }

    /// Save one prompt text and return its stable hash.
    pub fn save_prompt_blob(&self, text: &str) -> Result<String, CoreError> {
        let hash = Uuid::new_v5(&Uuid::NAMESPACE_URL, text.as_bytes()).to_string();
        let now = self.clock.now_micros();
        let text = text.to_owned();
        let write_hash = hash.clone();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            writer.execute(
                "INSERT OR IGNORE INTO prompt_blobs (hash, text, created_at) VALUES (?, ?, ?)",
                (write_hash.as_str(), text.as_str(), now),
            )?;
            Ok(())
        })?;
        Ok(hash)
    }

    /// Read the newest saved prompt.
    pub fn latest_prompt(&self) -> Result<Option<PromptBlob>, CoreError> {
        Ok(self.storage.read(|reader| {
            reader.query_optional(
                "SELECT hash, text, created_at FROM prompt_blobs \
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [],
                |row| {
                    Ok(PromptBlob {
                        hash: row.get(0)?,
                        text: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                },
            )
        })?)
    }

    /// Record one model call in the usage ledger.
    pub fn record_usage(&self, new: NewUsage) -> Result<UsageEvent, CoreError> {
        let id = self.ids.new_id();
        let ts = self.clock.now_micros();
        let event = UsageEvent {
            id: id.clone(),
            ts,
            task_id: new.task_id,
            conversation_id: new.conversation_id,
            provider: new.provider,
            model: new.model,
            kind: new.kind,
            input_tokens: new.usage.input_tokens,
            cache_read_tokens: new.usage.cache_read_tokens,
            cache_write_tokens: new.usage.cache_write_tokens,
            output_tokens: new.usage.output_tokens,
            reasoning_tokens: new.usage.reasoning_tokens,
            cost_micros: new.cost_micros,
            latency_ms: new.latency_ms,
        };
        let insert = event.clone();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            writer.execute(
                "INSERT INTO usage_events (id, ts, task_id, conversation_id, provider, model, \
                 kind, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, \
                 reasoning_tokens, cost_micros, latency_ms) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    insert.id.as_str(),
                    insert.ts,
                    insert.task_id.as_deref(),
                    insert.conversation_id.as_deref(),
                    insert.provider.as_str(),
                    insert.model.as_str(),
                    insert.kind.as_str(),
                    insert.input_tokens,
                    insert.cache_read_tokens,
                    insert.cache_write_tokens,
                    insert.output_tokens,
                    insert.reasoning_tokens,
                    insert.cost_micros,
                    insert.latency_ms,
                ),
            )?;
            Ok(())
        })?;
        Ok(event)
    }

    /// Read the usage recorded for a task, oldest first.
    pub fn usage_for_task(&self, task_id: &str) -> Result<Vec<UsageEvent>, CoreError> {
        let raw: Vec<UsageEvent> = self.storage.read(|reader| {
            reader.query(
                "SELECT id, ts, task_id, conversation_id, provider, model, kind, input_tokens, \
                 cache_read_tokens, cache_write_tokens, output_tokens, reasoning_tokens, \
                 cost_micros, latency_ms FROM usage_events WHERE task_id = ? ORDER BY ts, id",
                (task_id,),
                |row| {
                    Ok(UsageEvent {
                        id: row.get(0)?,
                        ts: row.get(1)?,
                        task_id: row.get(2)?,
                        conversation_id: row.get(3)?,
                        provider: row.get(4)?,
                        model: row.get(5)?,
                        kind: row.get(6)?,
                        input_tokens: row.get(7)?,
                        cache_read_tokens: row.get(8)?,
                        cache_write_tokens: row.get(9)?,
                        output_tokens: row.get(10)?,
                        reasoning_tokens: row.get(11)?,
                        cost_micros: row.get(12)?,
                        latency_ms: row.get(13)?,
                    })
                },
            )
        })?;
        Ok(raw)
    }

    /// Read the newest usage row.
    pub fn latest_usage(&self) -> Result<Option<UsageEvent>, CoreError> {
        let raw: Option<UsageEvent> = self.storage.read(|reader| {
            reader.query_optional(
                "SELECT id, ts, task_id, conversation_id, provider, model, kind, input_tokens, \
                 cache_read_tokens, cache_write_tokens, output_tokens, reasoning_tokens, \
                 cost_micros, latency_ms FROM usage_events ORDER BY ts DESC, id DESC LIMIT 1",
                [],
                |row| {
                    Ok(UsageEvent {
                        id: row.get(0)?,
                        ts: row.get(1)?,
                        task_id: row.get(2)?,
                        conversation_id: row.get(3)?,
                        provider: row.get(4)?,
                        model: row.get(5)?,
                        kind: row.get(6)?,
                        input_tokens: row.get(7)?,
                        cache_read_tokens: row.get(8)?,
                        cache_write_tokens: row.get(9)?,
                        output_tokens: row.get(10)?,
                        reasoning_tokens: row.get(11)?,
                        cost_micros: row.get(12)?,
                        latency_ms: row.get(13)?,
                    })
                },
            )
        })?;
        Ok(raw)
    }

    /// Count running and asleep tasks.
    pub fn task_counts(&self) -> Result<TaskCounts, CoreError> {
        let rows: Vec<(String, i64)> = self.storage.read(|reader| {
            reader.query(
                "SELECT status, COUNT(*) FROM tasks \
                 WHERE status IN ('running','asleep') GROUP BY status",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })?;
        let mut counts = TaskCounts::default();
        for (status, count) in rows {
            match status.as_str() {
                "running" => counts.running = count as usize,
                "asleep" => counts.asleep = count as usize,
                _ => {}
            }
        }
        Ok(counts)
    }

    /// Sum usage recorded at or after `ts` in UTC microseconds.
    pub fn usage_totals_since(&self, ts: i64) -> Result<UsageTotals, CoreError> {
        Ok(self.storage.read(|reader| {
            reader.query_row(
                "SELECT COALESCE(SUM(input_tokens),0), \
                 COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(cost_micros),0) \
                 FROM usage_events WHERE ts >= ?",
                (ts,),
                |row| {
                    Ok(UsageTotals {
                        input_tokens: row.get(0)?,
                        cache_read_tokens: row.get(1)?,
                        cost_micros: row.get(2)?,
                    })
                },
            )
        })?)
    }

    /// Count memory items, for status until memory arrives.
    pub fn memory_item_count(&self) -> Result<usize, CoreError> {
        let count: i64 = self.storage.read(|reader| {
            reader.query_row("SELECT COUNT(*) FROM memory_items", [], |row| row.get(0))
        })?;
        Ok(count as usize)
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

    /// Hand in one Telegram update with a receipt.
    pub fn submit_telegram(
        &self,
        conversation_id: &str,
        request_id: &str,
        content: Value,
        update: TelegramUpdate,
        next_offset: i64,
    ) -> Result<TelegramAdmission, CoreError> {
        let id = self.ids.new_id();
        let entry = NewEntry {
            kind: "user".to_owned(),
            content: content.clone(),
            source: Some(update.message_id.to_string()),
            prompt_hash: None,
        };
        let save = Save::new(conversation_id)
            .entry_with(entry)
            .submission(NewSubmission {
                id: id.clone(),
                request_id: request_id.to_owned(),
                kind: "input".to_owned(),
                content,
            })
            .telegram_update(update)
            .telegram_offset(next_offset);

        match self.save(save) {
            Ok(_) => {
                let submission = self
                    .submission_by_id(&id)?
                    .ok_or_else(|| CoreError::SubmissionNotFound(id.clone()))?;
                Ok(TelegramAdmission {
                    submission,
                    duplicate: false,
                })
            }
            Err(CoreError::Storage(error)) if error.is_unique_violation() => {
                match self.submission_by_request_id(request_id)? {
                    Some(submission) => Ok(TelegramAdmission {
                        submission,
                        duplicate: true,
                    }),
                    None => Err(CoreError::Storage(error)),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Whether a Telegram update id was already admitted.
    pub fn telegram_update_seen(&self, update_id: i64) -> Result<bool, CoreError> {
        let seen: Option<i64> = self.storage.read(|reader| {
            reader.query_optional(
                "SELECT 1 FROM telegram_updates WHERE update_id = ?",
                (update_id,),
                |row| row.get(0),
            )
        })?;
        Ok(seen.is_some())
    }

    /// Record a control update and its next offset without a submission.
    pub fn record_telegram_update(
        &self,
        update: TelegramUpdate,
        next_offset: i64,
    ) -> Result<bool, CoreError> {
        let now = self.clock.now_micros();
        self.storage
            .write(move |writer| -> Result<bool, CoreError> {
                let inserted = match writer.execute(
                    "INSERT INTO telegram_updates (update_id, chat_id, message_id, processed_at) \
                 VALUES (?, ?, ?, ?)",
                    (update.update_id, update.chat_id, update.message_id, now),
                ) {
                    Ok(_) => true,
                    Err(error) if error.is_unique_violation() => false,
                    Err(error) => return Err(error.into()),
                };
                if inserted {
                    writer.execute(
                        "INSERT INTO meta (key, value) VALUES ('telegram_offset', ?) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        (next_offset.to_string(),),
                    )?;
                }
                Ok(inserted)
            })
    }

    /// Count queued submissions in one conversation.
    pub fn queued_submission_count(&self, conversation_id: &str) -> Result<usize, CoreError> {
        let count: i64 = self.storage.read(|reader| {
            reader.query_row(
                "SELECT COUNT(*) FROM submissions WHERE conversation_id = ? AND status = 'queued'",
                (conversation_id,),
                |row| row.get(0),
            )
        })?;
        Ok(count.max(0) as usize)
    }

    /// The oldest queued Telegram submission.
    pub fn next_queued_telegram_submission(&self) -> Result<Option<Submission>, CoreError> {
        let raw: Option<RawSubmission> = self.storage.read(|reader| {
            reader.query_optional(
                "SELECT s.id, s.conversation_id, s.request_id, s.kind, s.content_json, \
                 s.status, s.answer_entry_id, s.error_json, s.created_at, s.settled_at \
                 FROM submissions s JOIN conversations c ON c.id = s.conversation_id \
                 WHERE s.status = 'queued' AND c.transport = 'telegram' \
                 ORDER BY s.created_at, s.id LIMIT 1",
                [],
                |row| {
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
                },
            )
        })?;
        raw.map(RawSubmission::into_submission).transpose()
    }

    /// Settle a submission exactly once.
    pub fn settle(
        &self,
        submission_id: &str,
        status: SubmissionStatus,
        answer_entry_id: Option<&str>,
    ) -> Result<Submission, CoreError> {
        self.settle_with_error(submission_id, status, answer_entry_id, None)
    }

    /// Settle a submission and record an optional structured reason.
    pub fn settle_with_error(
        &self,
        submission_id: &str,
        status: SubmissionStatus,
        answer_entry_id: Option<&str>,
        error: Option<Value>,
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

        let save = Save::new(&existing.conversation_id).settle_with_error(
            submission_id,
            status,
            answer_entry_id.map(str::to_owned),
            error,
        );
        self.save(save)?;
        self.submission_by_id(submission_id)?
            .ok_or_else(|| CoreError::SubmissionNotFound(submission_id.to_owned()))
    }

    /// Read the next Telegram update offset. Zero means no offset yet.
    pub fn telegram_offset(&self) -> Result<i64, CoreError> {
        let value: Option<String> = self.storage.read(|reader| {
            reader.query_optional(
                "SELECT value FROM meta WHERE key = 'telegram_offset'",
                [],
                |row| row.get(0),
            )
        })?;
        match value {
            Some(value) => value
                .parse()
                .map_err(|_| CoreError::Corrupt("telegram_offset is not a number".to_owned())),
            None => Ok(0),
        }
    }

    /// Store the next Telegram update offset.
    pub fn set_telegram_offset(&self, offset: i64) -> Result<(), CoreError> {
        self.storage.write(move |writer| -> Result<(), CoreError> {
            writer.execute(
                "INSERT INTO meta (key, value) VALUES ('telegram_offset', ?) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (offset.to_string(),),
            )?;
            Ok(())
        })?;
        Ok(())
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

    /// Read the newest conversation for a transport channel.
    pub fn conversation_by_transport(
        &self,
        transport: &str,
        key: &str,
    ) -> Result<Option<Conversation>, CoreError> {
        let sql = format!(
            "{CONVERSATION_SELECT} WHERE transport = ? AND transport_key = ? \
             AND archived = 0 ORDER BY created_at DESC, rowid DESC LIMIT 1"
        );
        let raw: Option<RawConversation> = self.storage.read(|reader| {
            reader.query_optional(&sql, (transport, key), |row| {
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

    /// Assemble the prompt for a conversation.
    pub fn build_prompt(&self, input: PromptInput) -> Result<Prompt, CoreError> {
        crate::prompt::build_for(self, input)
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

    /// Mark a queued submission as running. A running submission is idempotent.
    pub fn begin_submission(&self, id: &str) -> Result<Submission, CoreError> {
        let now = self.clock.now_micros();
        let submission_id = id.to_owned();
        let write_id = submission_id.clone();
        let changed = self
            .storage
            .write(move |writer| -> Result<bool, CoreError> {
                let changed = writer.execute(
                    "UPDATE submissions SET status = 'running' WHERE id = ? AND status = 'queued'",
                    (write_id.as_str(),),
                )?;
                if changed == 1 {
                    let conversation_id: String = writer.query_row(
                        "SELECT conversation_id FROM submissions WHERE id = ?",
                        (write_id.as_str(),),
                        |row| row.get(0),
                    )?;
                    record_commit(
                        writer,
                        now,
                        Some(conversation_id.as_str()),
                        None,
                        "submission.running",
                    )?;
                }
                Ok(changed == 1)
            })?;
        let submission = self
            .submission_by_id(&submission_id)?
            .ok_or_else(|| CoreError::SubmissionNotFound(submission_id.clone()))?;
        if changed || submission.status == SubmissionStatus::Running {
            return Ok(submission);
        }
        match submission.status {
            SubmissionStatus::Queued => Err(CoreError::Invalid(
                "submission could not be started".to_owned(),
            )),
            status => Err(CoreError::AlreadySettled(
                submission_id,
                status.as_str().to_owned(),
            )),
        }
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
            if let Some(conversation_id) = insert.conversation_id.as_deref() {
                let found: Option<i64> = writer.query_optional(
                    "SELECT 1 FROM conversations WHERE id = ?",
                    (conversation_id,),
                    |row| row.get(0),
                )?;
                if found.is_none() {
                    return Err(CoreError::ConversationNotFound(conversation_id.to_owned()));
                }
            }
            if let Some(parent_id) = insert.parent_task_id.as_deref() {
                let found: Option<i64> = writer.query_optional(
                    "SELECT 1 FROM tasks WHERE id = ?",
                    (parent_id,),
                    |row| row.get(0),
                )?;
                if found.is_none() {
                    return Err(CoreError::TaskNotFound(parent_id.to_owned()));
                }
            }
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
            record_commit(
                writer,
                insert.created_at,
                insert.conversation_id.as_deref(),
                Some(insert.id.as_str()),
                "task.create",
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
                    "SELECT id, conversation_id, kind, phase, attempt, input_json, checkpoint_json \
                     FROM tasks WHERE status = 'pending' \
                     AND (next_wake_at IS NULL OR next_wake_at <= ?) \
                     ORDER BY COALESCE(next_wake_at, created_at), id LIMIT 1",
                    (now,),
                    |row| {
                        Ok(RawClaim {
                            id: row.get(0)?,
                            conversation_id: row.get(1)?,
                            kind: row.get(2)?,
                            phase: row.get(3)?,
                            attempt: row.get(4)?,
                            input_json: row.get(5)?,
                            checkpoint_json: row.get(6)?,
                        })
                    },
                )?;
                let Some(row) = row else {
                    return Ok(None);
                };
                let attempt = row.attempt + 1;
                let changed = writer.execute(
                    "UPDATE tasks SET status = 'running', lease_owner = ?, lease_until = ?, \
                     attempt = ?, next_wake_at = NULL, updated_at = ? \
                     WHERE id = ? AND status = 'pending'",
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
                record_commit(
                    writer,
                    now,
                    row.conversation_id.as_deref(),
                    Some(row.id.as_str()),
                    "task.claim",
                )?;
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

    /// Claim one specific runnable task.
    pub fn claim_task_by_id(
        &self,
        task_id: &str,
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
        let task_id = task_id.to_owned();
        let owner = owner.to_owned();
        self.storage
            .write(move |writer| -> Result<Option<ClaimedTask>, CoreError> {
                let row: Option<RawClaim> = writer.query_optional(
                    "SELECT id, conversation_id, kind, phase, attempt, input_json, checkpoint_json \
                     FROM tasks WHERE id = ? AND status = 'pending' \
                     AND (next_wake_at IS NULL OR next_wake_at <= ?)",
                    (task_id.as_str(), now),
                    |row| {
                        Ok(RawClaim {
                            id: row.get(0)?,
                            conversation_id: row.get(1)?,
                            kind: row.get(2)?,
                            phase: row.get(3)?,
                            attempt: row.get(4)?,
                            input_json: row.get(5)?,
                            checkpoint_json: row.get(6)?,
                        })
                    },
                )?;
                let Some(row) = row else {
                    return Ok(None);
                };
                let attempt = row.attempt + 1;
                let changed = writer.execute(
                    "UPDATE tasks SET status = 'running', lease_owner = ?, lease_until = ?, \
                     attempt = ?, next_wake_at = NULL, updated_at = ? \
                     WHERE id = ? AND status = 'pending'",
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
                record_commit(
                    writer,
                    now,
                    row.conversation_id.as_deref(),
                    Some(row.id.as_str()),
                    "task.claim",
                )?;
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
        owner: &str,
        lease_micros: i64,
    ) -> Result<(), CoreError> {
        if owner.trim().is_empty() {
            return Err(CoreError::Invalid("task owner is empty".to_owned()));
        }
        if lease_micros <= 0 {
            return Err(CoreError::Invalid("task lease must be positive".to_owned()));
        }
        let now = self.clock.now_micros();
        let task_id = task_id.to_owned();
        let owner = owner.to_owned();
        self.storage.write(move |writer| -> Result<(), CoreError> {
            let changed = writer.execute(
                "UPDATE tasks SET lease_until = ?, updated_at = ? \
                 WHERE id = ? AND status = 'running' AND lease_owner = ?",
                (now + lease_micros, now, task_id.as_str(), owner.as_str()),
            )?;
            if changed == 1 {
                return Ok(());
            }
            let task = load_task(writer, &task_id)?
                .ok_or_else(|| CoreError::TaskNotFound(task_id.clone()))?;
            if task.status != TaskStatus::Running {
                return Err(CoreError::TaskNotRunning(
                    task_id,
                    task.status.as_str().to_owned(),
                ));
            }
            Err(CoreError::LeaseLost(task_id))
        })
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

                let kind = match &output {
                    StepOutput::Next { .. } => "task.next",
                    StepOutput::Sleep { .. } => "task.sleep",
                    StepOutput::Terminal { .. } => "task.terminal",
                    StepOutput::Failed {
                        retryable: true, ..
                    } => "task.retry",
                    StepOutput::Failed {
                        retryable: false, ..
                    } => "task.fail",
                };

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
                    StepOutput::Sleep {
                        until_micros,
                        phase,
                        checkpoint,
                    } => {
                        let checkpoint = serde_json::to_string(&checkpoint)
                            .map_err(|error| CoreError::Invalid(error.to_string()))?;
                        let changed = writer.execute(
                            "UPDATE tasks SET status = 'asleep', phase = ?, checkpoint_json = ?, \
                             attempt = 0, lease_owner = NULL, lease_until = NULL, \
                             next_wake_at = ?, updated_at = ? \
                             WHERE id = ? AND status = 'running' AND lease_owner = ?",
                            (
                                phase,
                                checkpoint,
                                until_micros,
                                now,
                                task_id.as_str(),
                                owner.as_str(),
                            ),
                        )?;
                        if changed != 1 {
                            return Err(CoreError::LeaseLost(task_id));
                        }
                    }
                    StepOutput::Terminal { outcome, result } => {
                        let status = outcome.status();
                        let result = serde_json::to_string(&result)
                            .map_err(|error| CoreError::Invalid(error.to_string()))?;
                        let changed = writer.execute(
                            "UPDATE tasks SET status = ?, result_json = ?, terminal_at = ?, \
                             lease_owner = NULL, lease_until = NULL, next_wake_at = NULL, \
                             updated_at = ? WHERE id = ? AND status = 'running' AND lease_owner = ?",
                            (
                                status.as_str(),
                                result,
                                now,
                                now,
                                task_id.as_str(),
                                owner.as_str(),
                            ),
                        )?;
                        if changed != 1 {
                            return Err(CoreError::LeaseLost(task_id));
                        }
                    }
                    StepOutput::Failed {
                        retryable: true, ..
                    } => {
                        let changed = writer.execute(
                            "UPDATE tasks SET status = 'pending', attempt = 0, \
                             lease_owner = NULL, lease_until = NULL, next_wake_at = ?, \
                             updated_at = ? WHERE id = ? AND status = 'running' \
                             AND lease_owner = ?",
                            (now + 1_000_000, now, task_id.as_str(), owner.as_str()),
                        )?;
                        if changed != 1 {
                            return Err(CoreError::LeaseLost(task_id));
                        }
                    }
                    StepOutput::Failed {
                        error,
                        retryable: false,
                    } => {
                        let result = serde_json::to_string(&error)
                            .map_err(|error| CoreError::Invalid(error.to_string()))?;
                        let changed = writer.execute(
                            "UPDATE tasks SET status = 'failed', result_json = ?, \
                             terminal_at = ?, lease_owner = NULL, lease_until = NULL, \
                             next_wake_at = NULL, updated_at = ? \
                             WHERE id = ? AND status = 'running' AND lease_owner = ?",
                            (
                                result,
                                now,
                                now,
                                task_id.as_str(),
                                owner.as_str(),
                            ),
                        )?;
                        if changed != 1 {
                            return Err(CoreError::LeaseLost(task_id));
                        }
                    }
                }

                record_commit(
                    writer,
                    now,
                    task.conversation_id.as_deref(),
                    Some(task_id.as_str()),
                    kind,
                )?;

                load_task(writer, &task_id)?.ok_or(CoreError::TaskNotFound(task_id))
            })
    }

    /// Save the checkpoint of a running task without releasing its lease.
    pub fn checkpoint_task(
        &self,
        task_id: &str,
        owner: &str,
        checkpoint: Value,
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
                let checkpoint = serde_json::to_string(&checkpoint)
                    .map_err(|error| CoreError::Invalid(error.to_string()))?;
                let changed = writer.execute(
                    "UPDATE tasks SET checkpoint_json = ?, updated_at = ? \
                     WHERE id = ? AND status = 'running' AND lease_owner = ?",
                    (checkpoint, now, task_id.as_str(), owner.as_str()),
                )?;
                if changed != 1 {
                    return Err(CoreError::LeaseLost(task_id));
                }
                record_commit(
                    writer,
                    now,
                    task.conversation_id.as_deref(),
                    Some(task_id.as_str()),
                    "task.checkpoint",
                )?;
                load_task(writer, &task_id)?.ok_or(CoreError::TaskNotFound(task_id))
            })
    }

    /// The running generation task for a conversation, if any.
    pub fn active_generation_task(&self, conversation_id: &str) -> Result<Option<Task>, CoreError> {
        let id: Option<String> = self.storage.read(|reader| {
            reader.query_optional(
                "SELECT id FROM tasks WHERE conversation_id = ? AND kind = 'generation' \
                 AND status = 'running' ORDER BY created_at, id LIMIT 1",
                (conversation_id,),
                |row| row.get(0),
            )
        })?;
        match id {
            Some(id) => self.task(&id),
            None => Ok(None),
        }
    }

    /// Requeue tasks whose leases or sleeps expired.
    pub fn recover_tasks(&self) -> Result<RecoveryReport, CoreError> {
        let now = self.clock.now_micros();
        self.storage
            .write(move |writer| -> Result<RecoveryReport, CoreError> {
                let mut report = RecoveryReport::default();
                let due: Vec<(String, Option<String>)> = writer.query(
                    "SELECT id, conversation_id FROM tasks WHERE status = 'asleep' \
                     AND next_wake_at IS NOT NULL AND next_wake_at <= ?",
                    (now,),
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                for (id, conversation_id) in due {
                    let changed = writer.execute(
                        "UPDATE tasks SET status = 'pending', next_wake_at = NULL, updated_at = ? \
                         WHERE id = ? AND status = 'asleep'",
                        (now, id.as_str()),
                    )?;
                    if changed == 1 {
                        record_commit(
                            writer,
                            now,
                            conversation_id.as_deref(),
                            Some(id.as_str()),
                            "task.wake",
                        )?;
                        report.woken += 1;
                    }
                }
                let expired: Vec<(String, i64, Option<String>)> = writer.query(
                    "SELECT id, attempt, conversation_id FROM tasks WHERE status = 'running' \
                     AND (lease_until IS NULL OR lease_until <= ?)",
                    (now,),
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                for (id, attempt, conversation_id) in expired {
                    if attempt >= 2 {
                        let result = serde_json::to_string(&serde_json::json!({
                            "error": "poisoned"
                        }))
                        .map_err(|error| CoreError::Invalid(error.to_string()))?;
                        let changed = writer.execute(
                            "UPDATE tasks SET status = 'failed', result_json = ?, \
                             terminal_at = ?, lease_owner = NULL, lease_until = NULL, \
                             next_wake_at = NULL, updated_at = ? \
                             WHERE id = ? AND status = 'running'",
                            (result, now, now, id.as_str()),
                        )?;
                        if changed == 1 {
                            record_commit(
                                writer,
                                now,
                                conversation_id.as_deref(),
                                Some(id.as_str()),
                                "task.poison",
                            )?;
                            report.poisoned += 1;
                        }
                    } else {
                        let changed = writer.execute(
                            "UPDATE tasks SET status = 'pending', lease_owner = NULL, \
                             lease_until = NULL, updated_at = ? \
                             WHERE id = ? AND status = 'running'",
                            (now, id.as_str()),
                        )?;
                        if changed == 1 {
                            record_commit(
                                writer,
                                now,
                                conversation_id.as_deref(),
                                Some(id.as_str()),
                                "task.recover",
                            )?;
                            report.requeued += 1;
                        }
                    }
                }
                Ok(report)
            })
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

                let seq = record_commit(writer, now, Some(fork.id.as_str()), None, "fork")?;
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

                let documents: Vec<(String, i64, String)> = writer.query(
                    "SELECT kind, version, json FROM documents WHERE conversation_id = ?",
                    (source_id.as_str(),),
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                for (kind, version, json) in documents {
                    writer.execute(
                        "INSERT INTO documents (conversation_id, kind, version, json, updated_at) \
                         VALUES (?, ?, ?, ?, ?)",
                        (fork.id.as_str(), kind.as_str(), version, json.as_str(), now),
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

struct RawEffect {
    id: String,
    task_id: Option<String>,
    kind: String,
    key: String,
    payload_json: String,
    status: String,
    result_json: Option<String>,
    attempts: i64,
    created_at: i64,
    settled_at: Option<i64>,
}

impl RawEffect {
    fn into_effect(self) -> Result<Effect, CoreError> {
        let status = EffectStatus::parse(&self.status)
            .ok_or_else(|| CoreError::Corrupt(format!("unknown effect status: {}", self.status)))?;
        let payload = serde_json::from_str(&self.payload_json)
            .map_err(|error| CoreError::Corrupt(error.to_string()))?;
        let result = self
            .result_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|error| CoreError::Corrupt(error.to_string()))?;
        Ok(Effect {
            id: self.id,
            task_id: self.task_id,
            kind: self.kind,
            key: self.key,
            payload,
            status,
            result,
            attempts: self.attempts,
            created_at: self.created_at,
            settled_at: self.settled_at,
        })
    }
}

struct RawClaim {
    id: String,
    conversation_id: Option<String>,
    kind: String,
    phase: String,
    attempt: i64,
    input_json: String,
    checkpoint_json: Option<String>,
}

fn next_sequence(writer: &Writer<'_>) -> Result<i64, CoreError> {
    Ok(
        writer.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM commits", [], |row| {
            row.get(0)
        })?,
    )
}

fn insert_commit(
    writer: &Writer<'_>,
    seq: i64,
    now: i64,
    conversation_id: Option<&str>,
    task_id: Option<&str>,
    kind: &str,
) -> Result<(), CoreError> {
    writer.execute(
        "INSERT INTO commits (seq, created_at, conversation_id, task_id, kind) \
         VALUES (?, ?, ?, ?, ?)",
        (seq, now, conversation_id, task_id, kind),
    )?;
    Ok(())
}

fn record_commit(
    writer: &Writer<'_>,
    now: i64,
    conversation_id: Option<&str>,
    task_id: Option<&str>,
    kind: &str,
) -> Result<i64, CoreError> {
    let seq = next_sequence(writer)?;
    insert_commit(writer, seq, now, conversation_id, task_id, kind)?;
    Ok(seq)
}

fn apply_memory(writer: &Writer<'_>, writes: &[MemoryWrite], now: i64) -> Result<(), CoreError> {
    for write in writes {
        match &write.change {
            MemoryChange::Add(item) => add_memory(writer, item, &write.actor, now)?,
            MemoryChange::Update(update) => update_memory(writer, update, &write.actor, now)?,
            MemoryChange::Forget { id } => forget_memory(writer, id, &write.actor, now)?,
            MemoryChange::Pin { id, standing } => {
                pin_memory(writer, id, standing, &write.actor, now)?
            }
        }
    }
    Ok(())
}

fn add_memory(
    writer: &Writer<'_>,
    item: &NewMemory,
    actor: &str,
    now: i64,
) -> Result<(), CoreError> {
    let tags =
        serde_json::to_string(&item.tags).map_err(|error| CoreError::Invalid(error.to_string()))?;
    writer.execute(
        "INSERT INTO memory_items (id, kind, content, tags, fingerprint, importance, status, \
         valid_from, valid_to, supersedes_id, source_kind, source_ref, standing, created_at, \
         last_used_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        (
            item.id.as_str(),
            item.kind.as_str(),
            item.content.as_str(),
            tags.as_str(),
            item.fingerprint.as_str(),
            item.importance,
            item.status.as_str(),
            item.valid_from,
            item.valid_to,
            item.supersedes_id.as_deref(),
            item.source_kind.as_str(),
            item.source_ref.as_deref(),
            item.standing.as_str(),
            item.created_at,
            item.last_used_at,
        ),
    )?;
    memory_event(
        writer,
        &item.id,
        "add",
        actor,
        None,
        Some(hash_content(&item.content)),
        now,
    )
}

struct OldMemory {
    content: String,
    tags: String,
    fingerprint: String,
    importance: i64,
    status: String,
    valid_to: Option<i64>,
    standing: String,
    last_used_at: Option<i64>,
}

fn update_memory(
    writer: &Writer<'_>,
    update: &MemoryUpdate,
    actor: &str,
    now: i64,
) -> Result<(), CoreError> {
    let old: Option<OldMemory> = writer.query_optional(
        "SELECT content, tags, fingerprint, importance, status, valid_to, standing, \
             last_used_at FROM memory_items WHERE id = ?",
        (update.id.as_str(),),
        |row| {
            Ok(OldMemory {
                content: row.get(0)?,
                tags: row.get(1)?,
                fingerprint: row.get(2)?,
                importance: row.get(3)?,
                status: row.get(4)?,
                valid_to: row.get(5)?,
                standing: row.get(6)?,
                last_used_at: row.get(7)?,
            })
        },
    )?;
    let Some(old) = old else {
        return Err(CoreError::Invalid(format!(
            "memory {} not found",
            update.id
        )));
    };
    let old_content = old.content;
    let old_tags = old.tags;
    let old_fingerprint = old.fingerprint;
    let old_importance = old.importance;
    let old_status = old.status;
    let old_valid_to = old.valid_to;
    let old_standing = old.standing;
    let old_used = old.last_used_at;
    let content = update
        .content
        .clone()
        .unwrap_or_else(|| old_content.clone());
    let tags = match &update.tags {
        Some(tags) => {
            serde_json::to_string(tags).map_err(|error| CoreError::Invalid(error.to_string()))?
        }
        None => old_tags.clone(),
    };
    let fingerprint = update.fingerprint.clone().unwrap_or_else(|| {
        if update.content.is_some() {
            hash_content(&content)
        } else {
            old_fingerprint.clone()
        }
    });
    let importance = update.importance.unwrap_or(old_importance);
    let status = update.status.clone().unwrap_or(old_status);
    let valid_to = update.valid_to.or(old_valid_to);
    let standing = update.standing.clone().unwrap_or(old_standing);
    let last_used_at = update.last_used_at.or(old_used);
    writer.execute(
        "UPDATE memory_items SET content = ?, tags = ?, fingerprint = ?, importance = ?, \
         status = ?, valid_to = ?, standing = ?, last_used_at = ? WHERE id = ?",
        (
            content.as_str(),
            tags.as_str(),
            fingerprint.as_str(),
            importance,
            status.as_str(),
            valid_to,
            standing.as_str(),
            last_used_at,
            update.id.as_str(),
        ),
    )?;
    memory_event(
        writer,
        &update.id,
        "update",
        actor,
        Some(hash_content(&old_content)),
        Some(hash_content(&content)),
        now,
    )
}

fn forget_memory(writer: &Writer<'_>, id: &str, actor: &str, now: i64) -> Result<(), CoreError> {
    let old: Option<(i64, String, String)> = writer.query_optional(
        "SELECT rowid, content, tags FROM memory_items WHERE id = ?",
        (id,),
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let Some((rowid, content, tags)) = old else {
        return Err(CoreError::Invalid(format!("memory {id} not found")));
    };
    writer.execute(
        "UPDATE memory_items SET status = 'deleted' WHERE id = ?",
        (id,),
    )?;
    writer.execute(
        "INSERT INTO memory_fts (memory_fts, rowid, content, tags) VALUES ('delete', ?, ?, ?)",
        (rowid, content.as_str(), tags.as_str()),
    )?;
    memory_event(
        writer,
        id,
        "forget",
        actor,
        Some(hash_content(&content)),
        None,
        now,
    )
}

fn pin_memory(
    writer: &Writer<'_>,
    id: &str,
    standing: &str,
    actor: &str,
    now: i64,
) -> Result<(), CoreError> {
    let content: Option<String> = writer.query_optional(
        "SELECT content FROM memory_items WHERE id = ?",
        (id,),
        |row| row.get(0),
    )?;
    let Some(content) = content else {
        return Err(CoreError::Invalid(format!("memory {id} not found")));
    };
    writer.execute(
        "UPDATE memory_items SET standing = ? WHERE id = ?",
        (standing, id),
    )?;
    memory_event(
        writer,
        id,
        "pin",
        actor,
        Some(hash_content(&content)),
        Some(hash_content(&content)),
        now,
    )
}

fn memory_event(
    writer: &Writer<'_>,
    item_id: &str,
    action: &str,
    actor: &str,
    old_hash: Option<String>,
    new_hash: Option<String>,
    now: i64,
) -> Result<(), CoreError> {
    writer.execute(
        "INSERT INTO memory_events (id, item_id, action, actor, old_hash, new_hash, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        (
            Uuid::now_v7().to_string(),
            item_id,
            action,
            actor,
            old_hash,
            new_hash,
            now,
        ),
    )?;
    Ok(())
}

fn hash_content(content: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, content.as_bytes()).to_string()
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

fn load_effect(writer: &Writer<'_>, id: &str) -> Result<Option<Effect>, CoreError> {
    let sql = format!("{EFFECT_SELECT} WHERE id = ?");
    let raw: Option<RawEffect> = writer.query_optional(&sql, (id,), |row| {
        Ok(RawEffect {
            id: row.get(0)?,
            task_id: row.get(1)?,
            kind: row.get(2)?,
            key: row.get(3)?,
            payload_json: row.get(4)?,
            status: row.get(5)?,
            result_json: row.get(6)?,
            attempts: row.get(7)?,
            created_at: row.get(8)?,
            settled_at: row.get(9)?,
        })
    })?;
    raw.map(RawEffect::into_effect).transpose()
}
