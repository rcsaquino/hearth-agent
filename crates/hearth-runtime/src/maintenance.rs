//! Nightly maintenance scheduling.

use std::path::PathBuf;
use std::sync::Arc;

use hearth_core::{ClaimedTask, CoreError, NewTask, Session, StepOutput, Zone};
use hearth_memory::MemoryService;
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::backup::{BackupError, backup};
use crate::events::{EventBus, RunEvent};
use crate::scheduler::{TaskHandler, TaskHandlerError};

/// The nightly database backup.
pub const BACKUP_KIND: &str = "backup";
pub const BACKUP_HOUR: u8 = 3;
pub const BACKUP_MINUTE: u8 = 0;

/// The scheduled skill review pass.
pub const SKILL_REVIEW_KIND: &str = "skill.review";
/// Skill review wakes at 04:30 local time.
pub const SKILL_REVIEW_HOUR: u8 = 4;
pub const SKILL_REVIEW_MINUTE: u8 = 30;

/// The scheduled memory audit and canonical duplicate repair.
pub const MEMORY_CONSOLIDATION_KIND: &str = "memory.consolidate";
pub const MEMORY_CONSOLIDATION_HOUR: u8 = 3;
pub const MEMORY_CONSOLIDATION_MINUTE: u8 = 30;

/// Helper-retention sweep.
pub const HELPER_RETENTION_KIND: &str = "helper.retention";
pub const HELPER_RETENTION_HOUR: u8 = 4;
pub const HELPER_RETENTION_MINUTE: u8 = 0;

/// Ensure the nightly maintenance tasks exist and are asleep.
pub fn ensure_maintenance_tasks(
    session: &Session,
    zone: &Zone,
    retention_days: u32,
    retention_runs: u32,
) -> Result<(), CoreError> {
    let now = session.now_micros();
    for (kind, slot, hour, minute) in [
        (BACKUP_KIND, "03:00", BACKUP_HOUR, BACKUP_MINUTE),
        (
            SKILL_REVIEW_KIND,
            "04:30",
            SKILL_REVIEW_HOUR,
            SKILL_REVIEW_MINUTE,
        ),
        (
            HELPER_RETENTION_KIND,
            "04:00",
            HELPER_RETENTION_HOUR,
            HELPER_RETENTION_MINUTE,
        ),
        (
            MEMORY_CONSOLIDATION_KIND,
            "03:30",
            MEMORY_CONSOLIDATION_HOUR,
            MEMORY_CONSOLIDATION_MINUTE,
        ),
    ] {
        if session.has_open_task(kind)? {
            continue;
        }
        let wake = zone.next_at(now, hour, minute);
        let checkpoint = if kind == HELPER_RETENTION_KIND {
            json!({
                "slot": slot,
                "retention_days": retention_days,
                "retention_runs": retention_runs,
            })
        } else {
            json!({"slot": slot})
        };
        session.create_sleeping_task(NewTask::new(kind, 1, "run", checkpoint), wake)?;
    }
    Ok(())
}

pub(crate) fn rfc3339(now_micros: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(now_micros as i128 * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// Deletes helper task records past the configured retention and archives the
/// helper conversations. Entries and report files remain part of the record.
pub struct HelperRetentionHandler {
    home: PathBuf,
    session: Session,
    retention_days: u32,
    retention_runs: u32,
}

impl HelperRetentionHandler {
    /// Build the handler for one home.
    pub fn new(
        home: impl Into<PathBuf>,
        session: Session,
        retention_days: u32,
        retention_runs: u32,
    ) -> Self {
        Self {
            home: home.into(),
            session,
            retention_days,
            retention_runs,
        }
    }
}

impl TaskHandler for HelperRetentionHandler {
    fn step(&self, _task: &ClaimedTask) -> Result<StepOutput, TaskHandlerError> {
        let now = self.session.now_micros();
        let zone = Zone::resolve(&hearth_config::identity_timezone(&self.home));
        let next = zone.next_at(now, HELPER_RETENTION_HOUR, HELPER_RETENTION_MINUTE);
        match self
            .session
            .prune_helpers(self.retention_days, self.retention_runs, now)
        {
            Ok(0) => {}
            Ok(deleted) => tracing::info!(deleted, "helper retention complete"),
            Err(error) => tracing::warn!(%error, "helper retention failed"),
        }
        Ok(StepOutput::Sleep {
            until_micros: next,
            phase: "run".to_owned(),
            checkpoint: json!({
                "slot": "04:00",
                "retention_days": self.retention_days,
                "retention_runs": self.retention_runs,
            }),
        })
    }
}

/// Audits memory, closes canonical duplicates, and sleeps until the next night.
pub struct MemoryConsolidationHandler {
    home: PathBuf,
    memory: Arc<MemoryService>,
    events: Option<Arc<EventBus>>,
}

impl MemoryConsolidationHandler {
    /// Build the handler for one home.
    pub fn new(
        home: impl Into<PathBuf>,
        memory: Arc<MemoryService>,
        events: Option<Arc<EventBus>>,
    ) -> Self {
        Self {
            home: home.into(),
            memory,
            events,
        }
    }
}

impl TaskHandler for MemoryConsolidationHandler {
    fn step(&self, task: &ClaimedTask) -> Result<StepOutput, TaskHandlerError> {
        let now = self.memory.session().now_micros();
        let zone = Zone::resolve(&hearth_config::identity_timezone(&self.home));
        let next = zone.next_at(now, MEMORY_CONSOLIDATION_HOUR, MEMORY_CONSOLIDATION_MINUTE);
        let mut checkpoint = json!({"slot": "03:30"});
        match self.memory.consolidate() {
            Ok(report) => {
                let summary = report.summary_json();
                checkpoint = json!({
                    "slot": "03:30",
                    "canonical_closed": report.canonical_closed,
                    "near_closed": report.near_closed,
                    "near_duplicates": report.near_duplicates,
                    "cross_subject": report.cross_subject,
                    "compound": report.compound,
                });
                if let Some(events) = &self.events {
                    events.publish(RunEvent {
                        task_id: task.id.clone(),
                        conversation_id: String::new(),
                        kind: "memory.consolidated",
                        data: summary,
                    });
                }
                tracing::info!(
                    closed = report.canonical_closed,
                    near_closed = report.near_closed,
                    near = report.near_duplicates,
                    "memory consolidation complete"
                );
            }
            Err(error) => tracing::warn!(%error, "memory consolidation failed"),
        }
        Ok(StepOutput::Sleep {
            until_micros: next,
            phase: "run".to_owned(),
            checkpoint,
        })
    }
}

/// Runs the nightly database backup and sleeps until the next night.
pub struct BackupHandler {
    home: PathBuf,
    session: Session,
}

impl BackupHandler {
    /// Build the handler for one home and its record.
    pub fn new(home: impl Into<PathBuf>, session: Session) -> Self {
        Self {
            home: home.into(),
            session,
        }
    }
}

impl TaskHandler for BackupHandler {
    fn step(&self, _task: &ClaimedTask) -> Result<StepOutput, TaskHandlerError> {
        let now = OffsetDateTime::now_utc()
            .unix_timestamp_nanos()
            .div_euclid(1_000) as i64;
        let zone = Zone::resolve(&hearth_config::identity_timezone(&self.home));
        let next = zone.next_at(now, BACKUP_HOUR, BACKUP_MINUTE);
        // The nightly pass also drops prompt blobs past their retention window,
        // before the snapshot, so backups stop carrying every past prompt.
        let cutoff = now - hearth_core::PROMPT_BLOB_RETENTION_MICROS;
        match self.session.prune_prompt_blobs(cutoff) {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "pruned old prompt blobs"),
            Err(error) => tracing::warn!(%error, "prompt blobs could not be pruned"),
        }
        let report = backup(&self.home).map_err(backup_handler_error)?;
        Ok(StepOutput::Sleep {
            until_micros: next,
            phase: "run".to_owned(),
            checkpoint: json!({
                "last_backup": rfc3339(now),
                "backup": report.path.to_string_lossy(),
            }),
        })
    }
}

fn backup_handler_error(error: BackupError) -> TaskHandlerError {
    let permanent = matches!(&error, BackupError::MissingDatabase(_));
    let message = error.to_string();
    if permanent {
        TaskHandlerError::Permanent(message)
    } else {
        TaskHandlerError::Retryable(message)
    }
}
