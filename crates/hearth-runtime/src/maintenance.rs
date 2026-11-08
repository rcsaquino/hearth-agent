//! Nightly maintenance scheduling and the memory consolidation handler.

use std::sync::Arc;

use hearth_core::{ClaimedTask, CoreError, NewTask, Session, StepOutput};
use hearth_memory::{MemoryError, MemoryService};
use serde_json::json;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, UtcOffset};

use crate::scheduler::{TaskHandler, TaskHandlerError};

/// The scheduled duplicate, archive, and episode pass.
pub const MEMORY_CONSOLIDATE_KIND: &str = "memory.consolidate";
/// The scheduled skill review pass.
pub const SKILL_REVIEW_KIND: &str = "skill.review";
/// Memory consolidation wakes at 03:30 local time.
pub const MEMORY_CONSOLIDATE_HOUR: u8 = 3;
pub const MEMORY_CONSOLIDATE_MINUTE: u8 = 30;
/// Skill review wakes at 04:30 local time.
pub const SKILL_REVIEW_HOUR: u8 = 4;
pub const SKILL_REVIEW_MINUTE: u8 = 30;

/// The host local offset, or UTC when the platform cannot answer.
pub fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

/// The next local hour:minute in UTC microseconds.
pub fn next_daily_micros(now_micros: i64, offset: UtcOffset, hour: u8, minute: u8) -> i64 {
    let now = OffsetDateTime::from_unix_timestamp_nanos(now_micros as i128 * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset);
    let Ok(primitive) = now.date().with_hms(hour, minute, 0) else {
        return now_micros;
    };
    let mut target = primitive.assume_offset(offset);
    if target <= now {
        target += Duration::days(1);
    }
    target
        .to_offset(UtcOffset::UTC)
        .unix_timestamp_nanos()
        .div_euclid(1_000) as i64
}

/// Ensure the two nightly maintenance tasks exist and are asleep.
pub fn ensure_maintenance_tasks(session: &Session) -> Result<(), CoreError> {
    let now = session.now_micros();
    let offset = local_offset();
    for (kind, slot, hour, minute) in [
        (
            MEMORY_CONSOLIDATE_KIND,
            "03:30",
            MEMORY_CONSOLIDATE_HOUR,
            MEMORY_CONSOLIDATE_MINUTE,
        ),
        (
            SKILL_REVIEW_KIND,
            "04:30",
            SKILL_REVIEW_HOUR,
            SKILL_REVIEW_MINUTE,
        ),
    ] {
        if session.has_open_task(kind)? {
            continue;
        }
        let wake = next_daily_micros(now, offset, hour, minute);
        session.create_sleeping_task(
            NewTask::new(kind, 1, "run", json!({"slot": slot})),
            wake,
        )?;
    }
    Ok(())
}

/// Runs the memory consolidation, archive, and episode pass, then sleeps until
/// the next night.
pub struct MemoryMaintenanceHandler {
    session: Session,
    memory: Arc<MemoryService>,
}

impl MemoryMaintenanceHandler {
    /// Build the handler.
    pub fn new(session: Session, memory: Arc<MemoryService>) -> Self {
        Self { session, memory }
    }
}

impl TaskHandler for MemoryMaintenanceHandler {
    fn step(&self, _task: &ClaimedTask) -> Result<StepOutput, TaskHandlerError> {
        let now = self.session.now_micros();
        let offset = local_offset();
        let next = next_daily_micros(
            now,
            offset,
            MEMORY_CONSOLIDATE_HOUR,
            MEMORY_CONSOLIDATE_MINUTE,
        );
        let merged = self
            .memory
            .consolidate()
            .map_err(memory_handler_error)?;
        let archived = self.memory.archive_stale().map_err(memory_handler_error)?;
        let episodes = self
            .memory
            .consolidate_episodes()
            .map_err(memory_handler_error)?;
        let stamp = rfc3339(now);
        self.session
            .set_meta("last_consolidation", &stamp)
            .map_err(|error| TaskHandlerError::Retryable(error.to_string()))?;
        Ok(StepOutput::Sleep {
            until_micros: next,
            phase: "run".to_owned(),
            checkpoint: json!({
                "last_consolidation": stamp,
                "merged": merged.merged,
                "unchanged": merged.unchanged,
                "archived": archived.archived,
                "episodes": episodes.written,
            }),
        })
    }
}

fn memory_handler_error(error: MemoryError) -> TaskHandlerError {
    let permanent = matches!(
        &error,
        MemoryError::Invalid(_)
            | MemoryError::Secret(_)
            | MemoryError::Corrupt(_)
            | MemoryError::NotFound(_)
    );
    let message = error.to_string();
    if permanent {
        TaskHandlerError::Permanent(message)
    } else {
        TaskHandlerError::Retryable(message)
    }
}

pub(crate) fn rfc3339(now_micros: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(now_micros as i128 * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned())
}
