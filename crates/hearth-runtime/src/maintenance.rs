//! Nightly maintenance scheduling.

use std::path::PathBuf;

use hearth_core::{ClaimedTask, CoreError, NewTask, Session, StepOutput, Zone};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::backup::{BackupError, backup};
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

/// Ensure the two nightly maintenance tasks exist and are asleep.
pub fn ensure_maintenance_tasks(session: &Session, zone: &Zone) -> Result<(), CoreError> {
    let now = session.now_micros();
    for (kind, slot, hour, minute) in [
        (BACKUP_KIND, "03:00", BACKUP_HOUR, BACKUP_MINUTE),
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
        let wake = zone.next_at(now, hour, minute);
        session.create_sleeping_task(NewTask::new(kind, 1, "run", json!({"slot": slot})), wake)?;
    }
    Ok(())
}

pub(crate) fn rfc3339(now_micros: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(now_micros as i128 * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// Runs the nightly database backup and sleeps until the next night.
pub struct BackupHandler {
    home: PathBuf,
}

impl BackupHandler {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
}

impl TaskHandler for BackupHandler {
    fn step(&self, _task: &ClaimedTask) -> Result<StepOutput, TaskHandlerError> {
        let now = OffsetDateTime::now_utc()
            .unix_timestamp_nanos()
            .div_euclid(1_000) as i64;
        let zone = Zone::resolve(&hearth_config::identity_timezone(&self.home));
        let next = zone.next_at(now, BACKUP_HOUR, BACKUP_MINUTE);
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
