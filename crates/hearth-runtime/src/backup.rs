//! Nightly and on-demand database backups, retention, and restore.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use hearth_storage::{snapshot, verify};
use thiserror::Error;
use time::{Date, Duration, Month, OffsetDateTime};

const DAILY_KEEP: usize = 7;
const WEEKLY_KEEP: usize = 4;

/// A backup failure.
#[derive(Debug, Error)]
pub enum BackupError {
    #[error(transparent)]
    Storage(#[from] hearth_storage::StorageError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("database not found at {0}; run hearth init")]
    MissingDatabase(PathBuf),
    #[error("backup not found: {0}")]
    MissingBackup(PathBuf),
    #[error("the daemon is running; stop it before restoring")]
    Running,
    #[error("invalid backup: {0}")]
    InvalidBackup(String),
}

/// The result of one backup.
#[derive(Debug, Clone)]
pub struct BackupReport {
    pub path: PathBuf,
    pub removed: Vec<PathBuf>,
}

/// The result of one restore.
#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub restored: PathBuf,
    pub pre_restore: Option<PathBuf>,
}

/// Snapshot the database into `backups/hearth-YYYYMMDD.db` and prune retention.
pub fn backup(home: &Path) -> Result<BackupReport, BackupError> {
    let database = home.join("hearth.db");
    if !database.is_file() {
        return Err(BackupError::MissingDatabase(database));
    }
    let backups = home.join("backups");
    fs::create_dir_all(&backups)?;
    let now = now_local();
    let today = now.date();
    let path = backups.join(backup_name(today));
    snapshot(&database, &path)?;
    let removed = prune(&backups, today)?;
    Ok(BackupReport { path, removed })
}

/// Prune snapshots to seven daily and four weekly copies.
pub fn prune(backups: &Path, today: Date) -> Result<Vec<PathBuf>, BackupError> {
    if !backups.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<(Date, PathBuf)> = Vec::new();
    for entry in fs::read_dir(backups)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(date) = parse_backup_name(name) else {
            continue;
        };
        if date > today {
            continue;
        }
        entries.push((date, entry.path()));
    }
    entries.sort_by(|left, right| right.0.cmp(&left.0));

    let daily: HashSet<PathBuf> = entries
        .iter()
        .take(DAILY_KEEP)
        .map(|(_, path)| path.clone())
        .collect();
    let mut weekly: BTreeMap<Date, PathBuf> = BTreeMap::new();
    for (date, path) in &entries {
        let weekday = date.weekday().number_days_from_monday() as i64;
        let monday = *date - Duration::days(weekday);
        weekly.entry(monday).or_insert_with(|| path.clone());
    }
    let mut keep = daily;
    keep.extend(weekly.values().rev().take(WEEKLY_KEEP).cloned());

    let mut removed = Vec::new();
    for (_, path) in entries {
        if !keep.contains(&path) {
            fs::remove_file(&path)?;
            removed.push(path);
        }
    }
    Ok(removed)
}

/// Replace the database with a verified backup, keeping a pre-restore copy.
pub fn restore(home: &Path, source: &Path) -> Result<RestoreReport, BackupError> {
    if !source.is_file() {
        return Err(BackupError::MissingBackup(source.to_path_buf()));
    }
    fs::create_dir_all(home)?;
    let lock = open_lock(&home.join("hearth.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(BackupError::Running),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }

    verify(source).map_err(|error| BackupError::InvalidBackup(error.to_string()))?;

    let database = home.join("hearth.db");
    let pre_restore = if database.is_file() {
        let backups = home.join("backups");
        fs::create_dir_all(&backups)?;
        let path = backups.join(format!("pre-restore-{}.db", timestamp(now_local())));
        snapshot(&database, &path)?;
        Some(path)
    } else {
        None
    };

    let temporary = home.join(".hearth-restore.tmp");
    let _ = fs::remove_file(&temporary);
    fs::copy(source, &temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    {
        let file = OpenOptions::new().write(true).open(&temporary)?;
        file.sync_all()?;
    }
    let _ = fs::remove_file(home.join("hearth.db-wal"));
    let _ = fs::remove_file(home.join("hearth.db-shm"));
    if let Err(error) = fs::rename(&temporary, &database) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    #[cfg(unix)]
    if let Ok(directory) = File::open(home) {
        let _ = directory.sync_all();
    }
    verify(&database).map_err(|error| BackupError::InvalidBackup(error.to_string()))?;
    drop(lock);
    Ok(RestoreReport {
        restored: database,
        pre_restore,
    })
}

fn backup_name(date: Date) -> String {
    format!(
        "hearth-{:04}{:02}{:02}.db",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn parse_backup_name(name: &str) -> Option<Date> {
    let digits = name.strip_prefix("hearth-")?.strip_suffix(".db")?;
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = digits[0..4].parse::<i32>().ok()?;
    let month = digits[4..6].parse::<u8>().ok()?;
    let day = digits[6..8].parse::<u8>().ok()?;
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

fn timestamp(now: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn now_local() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

fn open_lock(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
