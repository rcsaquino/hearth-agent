//! The `standing.md` standing layer, ported from `the standing store`.
//!
//! Hot memory is a single plain-text file of bullets:
//!
//! ```text
//! - [s_9f2c...] [p=80] Never use em dashes in output.
//! ```
//!
//! It is injected before every prompt, so it is reserved for standing
//! instructions that need no retrieval cue. The whole file is capped at
//! [`STANDING_LIMIT`] characters of serialized text, and the only way to make room is
//! to displace a *strictly lower* priority bullet, which is archived to SQLite
//! rather than dropped.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::errors::{ErrorCode, ExpectedError, Result};
use crate::files::{atomic_write, is_missing};
use crate::text::{MEMORY_LIMIT, STANDING_LIMIT, integer, single_line};

/// One bullet in `standing.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingItem {
    pub id: String,
    pub priority: i64,
    pub content: String,
}

/// The state of the standing file after an operation.
#[derive(Debug, Clone)]
pub struct StandingSnapshot {
    /// The serialized file contents.
    pub text: String,
    pub entries: Vec<StandingItem>,
    pub characters: usize,
    pub remaining: usize,
    /// Bullets evicted to make room, now archived in SQLite.
    pub archived: Vec<StandingItem>,
    /// Path of the pre-repair backup, when the file had to be trimmed on load.
    pub backup: Option<PathBuf>,
}

/// Result of adding a bullet.
#[derive(Debug, Clone)]
pub struct StandingAddResult {
    pub snapshot: StandingSnapshot,
    pub memory: StandingItem,
    /// False when identical content already existed and was reused unchanged.
    pub created: bool,
    pub note: Option<String>,
}

/// Result of editing a bullet.
#[derive(Debug, Clone)]
pub struct StandingEditResult {
    pub snapshot: StandingSnapshot,
    pub memory: StandingItem,
}

/// Result of deleting a bullet.
#[derive(Debug, Clone)]
pub struct StandingDeleteResult {
    pub snapshot: StandingSnapshot,
    pub deleted: StandingItem,
}

/// Serialize bullets back to file text.
fn serialize(entries: &[StandingItem]) -> String {
    entries
        .iter()
        .map(|entry| {
            format!(
                "- [{}] [p={}] {}\n",
                entry.id, entry.priority, entry.content
            )
        })
        .collect()
}

/// Parse file text into bullets.
///
/// Manual edits are tolerated: bare `- text` lines get a content-derived id at
/// priority 50, indented continuation lines are folded into the previous bullet,
/// and a duplicated hand-written line is merged when the content is identical.
/// An explicit duplicate id is still an error, because that means the file was
/// edited wrongly and silently picking a winner would lose a fact.
fn parse(text: &str) -> Result<Vec<StandingItem>> {
    let mut entries: Vec<StandingItem> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        // A Markdown heading labels a section; it is not a memory bullet.
        if trimmed.starts_with('#') {
            continue;
        }
        // An indented line continues the previous bullet.
        if line.starts_with(char::is_whitespace) && !trimmed.is_empty() && !entries.is_empty() {
            let last = entries.last_mut().expect("checked non-empty");
            last.content.push(' ');
            last.content.push_str(trimmed);
            continue;
        }
        let body = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
            .unwrap_or(trimmed)
            .trim();
        if body.is_empty() {
            continue;
        }
        let (id, priority, content) = match parse_metadata(body) {
            Some((id, priority, content)) => (id, priority, content),
            None => {
                let mut hasher = Sha256::new();
                hasher.update(body.as_bytes());
                let digest = format!("{:x}", hasher.finalize());
                (format!("s_{}", &digest[..16]), 50, body.to_owned())
            }
        };
        let explicit = parse_metadata(body).is_some();
        if let Some(existing) = entries.iter().find(|entry| entry.id == id) {
            // A repeated hand-written line with the same text is harmless.
            if !explicit && existing.content == content {
                continue;
            }
            return Err(ExpectedError::with_details(
                ErrorCode::InvalidInput,
                format!("Duplicate standing memory ID {id}; repair standing.md before editing."),
                serde_json::json!({ "id": id }),
            ));
        }
        entries.push(StandingItem {
            id,
            priority,
            content,
        });
    }
    Ok(entries)
}

/// Match `[s_id] [p=N] content`, returning the parsed fields.
fn parse_metadata(body: &str) -> Option<(String, i64, String)> {
    let rest = body.strip_prefix('[')?;
    let close = rest.find(']')?;
    let id = &rest[..close];
    if !id.starts_with("s_") || id.len() < 3 {
        return None;
    }
    if !id[2..]
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
    {
        return None;
    }
    let rest = rest[close + 1..].trim_start();
    let rest = rest.strip_prefix("[p=")?;
    let close = rest.find(']')?;
    let priority: i64 = rest[..close].parse().ok()?;
    if !(0..=100).contains(&priority) {
        return None;
    }
    let content = rest[close + 1..].trim_start();
    if content.is_empty() {
        return None;
    }
    Some((id.to_owned(), priority, content.to_owned()))
}

/// One bullet's size, as reported in capacity diagnostics.
fn described(entry: &StandingItem) -> serde_json::Value {
    serde_json::json!({
        "id": entry.id,
        "priority": entry.priority,
        "characters": serialize(std::slice::from_ref(entry)).chars().count(),
        "content": entry.content,
    })
}

/// Exact serialized diagnostics for a refused standing operation.
///
/// The original returns this shape so a caller can see precisely how much room
/// is missing and which bullets could be displaced. Keeping it means a failure
/// is actionable rather than just "no".
fn capacity_details(
    reason: &str,
    operation: &str,
    current: &[StandingItem],
    proposed: &[StandingItem],
    entry: &StandingItem,
) -> serde_json::Value {
    let current_characters = serialize(current).chars().count();
    let proposed_characters = serialize(proposed).chars().count();
    let mut eligible: Vec<&StandingItem> = current
        .iter()
        .filter(|item| item.id != entry.id && item.priority < entry.priority)
        .collect();
    eligible.sort_by_key(|item| item.priority);
    let eligible_characters: usize = eligible
        .iter()
        .map(|item| serialize(std::slice::from_ref(item)).chars().count())
        .sum();
    serde_json::json!({
        "reason": reason,
        "operation": operation,
        "limit": STANDING_LIMIT,
        "current_characters": current_characters,
        "proposed_characters": proposed_characters,
        "over_by": proposed_characters.saturating_sub(STANDING_LIMIT),
        "remaining": STANDING_LIMIT.saturating_sub(current_characters),
        "proposed_entry": described(entry),
        "entries": current.iter().map(described).collect::<Vec<_>>(),
        "eligible": eligible.iter().map(|item| described(item)).collect::<Vec<_>>(),
        "eligible_characters": eligible_characters,
        "shortfall_after_eligible_eviction": proposed_characters.saturating_sub(eligible_characters + STANDING_LIMIT),
        "live_file_unchanged": true,
        "next_actions": [
            "Shorten the bullet or explicitly delete lower-value bullets with memory_delete.",
            "Save the fact in SQLite long-term memory instead.",
            "Use memory_edit to raise the priority of an important entry only when justified."
        ],
    })
}

fn capacity_error(
    reason: &str,
    operation: &str,
    current: &[StandingItem],
    proposed: &[StandingItem],
    entry: &StandingItem,
) -> ExpectedError {
    let details = capacity_details(reason, operation, current, proposed, entry);
    let prefix = match reason {
        "bullet_too_large" => format!(
            "A single standing bullet serializes to {} characters including metadata, over the {STANDING_LIMIT}-character total limit.",
            details["proposed_entry"]["characters"]
                .as_u64()
                .unwrap_or(0)
        ),
        "existing_over_limit" => format!(
            "standing.md is already over the {STANDING_LIMIT}-character limit and must be repaired before duplicate content can be returned safely."
        ),
        _ => format!(
            "standing.md is {} characters over budget even after evicting every eligible lower-priority entry (shortfall {}).",
            details["over_by"].as_u64().unwrap_or(0),
            details["shortfall_after_eligible_eviction"]
                .as_u64()
                .unwrap_or(0)
        ),
    };
    ExpectedError::with_details(
        ErrorCode::StandingCapacity,
        format!(
            "{prefix} The live file was not changed. Shorten the content, delete a bullet, use SQLite, or adjust priority."
        ),
        details,
    )
}

/// A new bullet id, matching the original's `s_` plus 16 hex characters.
fn new_id() -> String {
    format!(
        "s_{}",
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(16)
            .collect::<String>()
    )
}

/// The archive sink for evicted bullets.
pub type ArchiveFn = dyn Fn(&[StandingItem]) -> Result<()> + Send + Sync;

/// The `standing.md` store.
///
/// Holds an in-process mutex so two turns in the same daemon cannot interleave a
/// read-modify-write, and a lock file so a second process cannot either.
pub struct StandingStore {
    path: PathBuf,
    archive: std::sync::Arc<ArchiveFn>,
    guard: Mutex<()>,
}

impl StandingStore {
    /// Build a store over one path, with a sink for archived bullets.
    pub fn new(path: impl Into<PathBuf>, archive: std::sync::Arc<ArchiveFn>) -> Self {
        Self {
            path: path.into(),
            archive,
            guard: Mutex::new(()),
        }
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Take the in-process and on-disk locks for the duration of `work`.
    fn locked<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        let _guard = self.guard.lock().map_err(|_| {
            ExpectedError::new(ErrorCode::StandingLocked, "standing memory lock poisoned")
        })?;
        let _file_lock = FileLock::acquire(&self.path)?;
        work()
    }

    /// Refuse to operate through a symlink.
    ///
    /// `standing.md` is replaced by rename, so a symlink would be swapped for a
    /// regular file and its target would then be read as prompt input. That is a
    /// quiet way to pull an arbitrary file into the prompt, so it is rejected.
    fn assert_plain_file(&self) -> Result<()> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(ExpectedError::with_details(
                ErrorCode::InvalidInput,
                format!(
                    "standing.md must be a regular file, not a symbolic link: {}",
                    self.path.display()
                ),
                serde_json::json!({ "path": self.path.display().to_string() }),
            )),
            Ok(_) => Ok(()),
            Err(error) if is_missing(&error) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn read(&self) -> Result<Option<String>> {
        self.assert_plain_file()?;
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if is_missing(&error) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write(&self, text: &str) -> Result<()> {
        self.assert_plain_file()?;
        atomic_write(&self.path, text)
    }

    fn snapshot(
        &self,
        entries: Vec<StandingItem>,
        archived: Vec<StandingItem>,
        backup: Option<PathBuf>,
    ) -> StandingSnapshot {
        let text = serialize(&entries);
        let characters = text.chars().count();
        StandingSnapshot {
            text,
            entries,
            characters,
            remaining: STANDING_LIMIT.saturating_sub(characters),
            archived,
            backup,
        }
    }

    /// Normalize manual edits and repair an over-budget file.
    ///
    /// Trimming happens lowest priority first. The pre-repair file is copied to
    /// `backups/` before anything is written, and the evicted bullets are
    /// archived to SQLite, so a hand-edit mistake is recoverable.
    pub fn load(&self) -> Result<StandingSnapshot> {
        self.locked(|| {
            let original = self.read()?;
            let previous = original.clone().unwrap_or_default();
            let mut entries = parse(&previous)?;
            let mut archived = Vec::new();
            while serialize(&entries).chars().count() > STANDING_LIMIT {
                let victim = entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.priority)
                    .map(|(index, _)| index)
                    .expect("non-empty when over budget");
                archived.push(entries.remove(victim));
            }
            let mut backup = None;
            if !archived.is_empty() {
                let directory = self
                    .path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("backups");
                fs::create_dir_all(&directory)?;
                let name = format!(
                    "MEMORY-{}-{}.md",
                    time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000,
                    uuid::Uuid::new_v4()
                );
                let path = directory.join(name);
                fs::write(&path, &previous)?;
                backup = Some(path);
                (self.archive)(&archived)?;
            }
            let result = self.snapshot(entries, archived, backup);
            // Ensure a real file exists even when the store is empty.
            if result.text != previous || original.is_none() {
                self.write(&result.text)?;
            }
            Ok(result)
        })
    }

    /// The current bullets, without repairing or writing anything.
    pub fn entries(&self) -> Result<Vec<StandingItem>> {
        parse(&self.read()?.unwrap_or_default())
    }

    /// The text injected into the prompt.
    pub fn text(&self) -> Result<String> {
        Ok(serialize(&self.entries()?))
    }

    /// Add a bullet, reusing an identical existing one.
    pub fn add(&self, content: &str, priority: i64) -> Result<StandingAddResult> {
        let normalized = single_line(content, "content", MEMORY_LIMIT)?;
        let requested = integer(priority, "priority", 0, 100)?;
        self.locked(|| {
            let entries = parse(&self.read()?.unwrap_or_default())?;
            // Clone the match so the borrow ends before `entries` is moved into
            // the snapshot below.
            let existing = entries
                .iter()
                .find(|entry| entry.content == normalized)
                .cloned();
            if let Some(existing) = existing {
                if serialize(&entries).chars().count() > STANDING_LIMIT {
                    return Err(capacity_error(
                        "existing_over_limit",
                        "add",
                        &entries,
                        &entries,
                        &existing,
                    ));
                }
                return Ok(StandingAddResult {
                    snapshot: self.snapshot(entries, Vec::new(), None),
                    memory: existing.clone(),
                    created: false,
                    note: Some(format!(
                        "Reused existing standing memory {}; its content, priority {}, and file position were preserved. Use memory_edit to change its content or priority.",
                        existing.id, existing.priority
                    )),
                });
            }
            let entry = StandingItem {
                id: new_id(),
                priority: requested,
                content: normalized,
            };
            let snapshot = self.commit(&entries, &entry, None, "add")?;
            Ok(StandingAddResult {
                snapshot,
                memory: entry,
                created: true,
                note: None,
            })
        })
    }

    /// Edit a bullet by id, preserving unspecified fields.
    pub fn edit(
        &self,
        id: &str,
        content: Option<&str>,
        priority: Option<i64>,
        expected_content: Option<&str>,
    ) -> Result<StandingEditResult> {
        let normalized = match content {
            Some(value) => Some(single_line(value, "content", MEMORY_LIMIT)?),
            None => None,
        };
        let requested = match priority {
            Some(value) => Some(integer(value, "priority", 0, 100)?),
            None => None,
        };
        self.locked(|| {
            let entries = parse(&self.read()?.unwrap_or_default())?;
            let index = entries.iter().position(|entry| entry.id == id).ok_or_else(|| {
                ExpectedError::with_details(
                    ErrorCode::NotFound,
                    format!("Hot memory {id} does not exist. Read standing.md again."),
                    serde_json::json!({ "id": id }),
                )
            })?;
            let previous = entries[index].clone();
            if let Some(expected) = expected_content
                && previous.content != expected
            {
                return Err(ExpectedError::with_details(
                    ErrorCode::Conflict,
                    format!(
                        "Hot memory {id} changed (stored content differs from expected_content). Read standing.md again before editing."
                    ),
                    serde_json::json!({
                        "id": id,
                        "expected_content": expected,
                        "current_content": previous.content,
                    }),
                ));
            }
            let entry = StandingItem {
                id: previous.id.clone(),
                content: normalized.clone().unwrap_or(previous.content.clone()),
                priority: requested.unwrap_or(previous.priority),
            };
            if let Some(normalized) = &normalized
                && *normalized != previous.content
                && let Some(duplicate) = entries
                    .iter()
                    .find(|item| item.id != id && &item.content == normalized)
            {
                return Err(ExpectedError::with_details(
                    ErrorCode::DuplicateStandingContent,
                    format!(
                        "Edit rejected: setting {id} to that content would duplicate standing memory {}. Neither entry was changed; keep one entry or choose distinct content.",
                        duplicate.id
                    ),
                    serde_json::json!({ "id": id, "existing_id": duplicate.id }),
                ));
            }
            let snapshot = self.commit(&entries, &entry, Some(index), "edit")?;
            Ok(StandingEditResult {
                snapshot,
                memory: entry,
            })
        })
    }

    /// Delete a bullet by id.
    pub fn delete(&self, id: &str, expected_content: Option<&str>) -> Result<StandingDeleteResult> {
        self.locked(|| {
            let mut entries = parse(&self.read()?.unwrap_or_default())?;
            let index = entries.iter().position(|entry| entry.id == id).ok_or_else(|| {
                ExpectedError::with_details(
                    ErrorCode::NotFound,
                    format!("Hot memory {id} does not exist."),
                    serde_json::json!({ "id": id }),
                )
            })?;
            let deleted = entries[index].clone();
            if let Some(expected) = expected_content
                && deleted.content != expected
            {
                return Err(ExpectedError::with_details(
                    ErrorCode::Conflict,
                    format!(
                        "Hot memory {id} changed (stored content differs from expected_content). Read standing.md again before deleting."
                    ),
                    serde_json::json!({
                        "id": id,
                        "expected_content": expected,
                        "current_content": deleted.content,
                    }),
                ));
            }
            entries.remove(index);
            let snapshot = self.snapshot(entries, Vec::new(), None);
            self.write(&snapshot.text)?;
            Ok(StandingDeleteResult { snapshot, deleted })
        })
    }

    /// Apply a proposed change, evicting lower-priority bullets to make room.
    fn commit(
        &self,
        before: &[StandingItem],
        entry: &StandingItem,
        index: Option<usize>,
        operation: &str,
    ) -> Result<StandingSnapshot> {
        let mut proposed = before.to_vec();
        match index {
            Some(index) => proposed[index] = entry.clone(),
            None => proposed.push(entry.clone()),
        }
        if serialize(std::slice::from_ref(entry)).chars().count() > STANDING_LIMIT {
            return Err(capacity_error(
                "bullet_too_large",
                operation,
                before,
                &proposed,
                entry,
            ));
        }
        let mut entries = proposed.clone();
        let mut archived = Vec::new();
        if serialize(&entries).chars().count() > STANDING_LIMIT {
            let mut candidates: Vec<StandingItem> = before
                .iter()
                .filter(|item| item.id != entry.id && item.priority < entry.priority)
                .cloned()
                .collect();
            candidates.sort_by_key(|item| item.priority);
            while serialize(&entries).chars().count() > STANDING_LIMIT {
                let Some(victim) = candidates.first().cloned() else {
                    return Err(capacity_error(
                        "cannot_free_space",
                        operation,
                        before,
                        &proposed,
                        entry,
                    ));
                };
                candidates.remove(0);
                archived.push(victim.clone());
                entries.retain(|item| item.id != victim.id);
            }
        }
        if !archived.is_empty() {
            (self.archive)(&archived)?;
        }
        let snapshot = self.snapshot(entries, archived, None);
        self.write(&snapshot.text)?;
        Ok(snapshot)
    }
}

/// A cross-process lock file beside `standing.md`.
///
/// The original uses `proper-lockfile` with a 10-second stale window and bounded
/// retries. This keeps the same window: a lock older than ten seconds is treated
/// as abandoned, which is what keeps a crashed process from wedging the file.
struct FileLock {
    path: PathBuf,
}

impl FileLock {
    fn acquire(target: &Path) -> Result<Self> {
        let path = target.with_extension("md.lock");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let _ = write!(file, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Steal a stale lock rather than hanging on a dead process.
                    if let Ok(metadata) = fs::metadata(&path)
                        && let Ok(modified) = metadata.modified()
                        && modified.elapsed().unwrap_or_default()
                            > std::time::Duration::from_secs(10)
                    {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(ExpectedError::with_details(
                            ErrorCode::StandingLocked,
                            "Could not lock standing.md within the retry window. Another process is mutating it; retry shortly.",
                            serde_json::json!({ "path": target.display().to_string() }),
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, StandingStore) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("standing.md");
        let archived = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&archived);
        let store = StandingStore::new(
            &path,
            std::sync::Arc::new(move |entries: &[StandingItem]| {
                sink.lock().unwrap().extend(entries.iter().cloned());
                Ok(())
            }),
        );
        (directory, store)
    }

    #[test]
    fn a_new_bullet_is_serialized_in_the_original_format() {
        let (_directory, store) = store();
        let result = store.add("Never use em dashes.", 90).unwrap();
        assert!(result.created);
        assert!(result.memory.id.starts_with("s_"));
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(text.starts_with(&format!(
            "- [{}] [p=90] Never use em dashes.\n",
            result.memory.id
        )));
    }

    #[test]
    fn identical_content_is_reused_not_duplicated() {
        let (_directory, store) = store();
        let first = store.add("Same fact", 50).unwrap();
        let second = store.add("Same fact", 80).unwrap();
        assert!(!second.created);
        assert_eq!(first.memory.id, second.memory.id);
        // The original explicitly preserves the existing priority.
        assert_eq!(second.memory.priority, 50);
        assert_eq!(store.entries().unwrap().len(), 1);
    }

    #[test]
    fn a_credential_like_bullet_is_refused() {
        let (_directory, store) = store();
        let error = store.add("token=abcdefghijk", 50).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("credential"), "{}", error.message);
    }

    #[test]
    fn a_multiline_bullet_is_rejected() {
        let (_directory, store) = store();
        assert_eq!(
            store.add("one\ntwo", 50).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn a_lower_priority_bullet_is_evicted_to_make_room() {
        let (_directory, store) = store();
        store.add("low value", 10).unwrap();
        // Large enough that, with the metadata overhead of both bullets, the file
        // must exceed STANDING_LIMIT and therefore needs an eviction.
        let long = "x".repeat(STANDING_LIMIT - 50);
        let result = store.add(&long, 90).unwrap();
        assert_eq!(result.snapshot.archived.len(), 1);
        assert_eq!(result.snapshot.archived[0].content, "low value");
        assert!(
            store
                .entries()
                .unwrap()
                .iter()
                .all(|entry| entry.content != "low value")
        );
    }

    #[test]
    fn a_higher_priority_bullet_is_never_evicted() {
        let (_directory, store) = store();
        store.add(&"x".repeat(STANDING_LIMIT - 200), 90).unwrap();
        let error = store.add(&"y".repeat(400), 10).unwrap_err();
        assert_eq!(error.code, ErrorCode::StandingCapacity);
        assert_eq!(error.details["reason"], "cannot_free_space");
        assert_eq!(error.details["live_file_unchanged"], true);
    }

    #[test]
    fn an_oversized_single_bullet_is_refused() {
        let (_directory, store) = store();
        let error = store.add(&"x".repeat(STANDING_LIMIT + 1), 50).unwrap_err();
        assert_eq!(error.code, ErrorCode::StandingCapacity);
        assert_eq!(error.details["reason"], "bullet_too_large");
    }

    #[test]
    fn a_stale_edit_is_rejected() {
        let (_directory, store) = store();
        let added = store.add("original", 50).unwrap();
        let error = store
            .edit(&added.memory.id, Some("changed"), None, Some("wrong"))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
    }

    #[test]
    fn an_edit_that_would_duplicate_is_rejected() {
        let (_directory, store) = store();
        store.add("first", 50).unwrap();
        let second = store.add("second", 50).unwrap();
        let error = store
            .edit(&second.memory.id, Some("first"), None, None)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::DuplicateStandingContent);
    }

    #[test]
    fn a_hand_written_bullet_gains_a_content_derived_id() {
        let (_directory, store) = store();
        fs::write(
            store.path(),
            "# standing.md\n\n- I prefer plain English.\n- [s_abc] [p=70] Explicit.\n",
        )
        .unwrap();
        let entries = store.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].priority, 50);
        assert_eq!(entries[0].content, "I prefer plain English.");
        assert!(entries[0].id.starts_with("s_"));
        assert_eq!(entries[1].priority, 70);
    }

    #[test]
    fn continuation_lines_are_folded_into_the_previous_bullet() {
        let (_directory, store) = store();
        fs::write(store.path(), "- first part\n  and the rest\n- second\n").unwrap();
        let entries = store.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "first part and the rest");
    }

    #[test]
    fn load_repairs_an_over_budget_file_and_keeps_a_backup() {
        let (_directory, store) = store();
        let mut text = String::new();
        for index in 0..40 {
            text.push_str(&format!(
                "- [s_{index:04}] [p={}] {}\n",
                index % 100,
                "x".repeat(300)
            ));
        }
        fs::write(store.path(), &text).unwrap();
        let snapshot = store.load().unwrap();
        assert!(snapshot.characters <= STANDING_LIMIT);
        assert!(!snapshot.archived.is_empty());
        let backup = snapshot.backup.expect("backup written");
        assert_eq!(fs::read_to_string(backup).unwrap(), text);
    }

    #[test]
    fn an_empty_store_materializes_a_file() {
        let (_directory, store) = store();
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.text, "");
        assert!(store.path().exists());
    }

    #[test]
    fn a_symlinked_file_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("elsewhere.txt");
        fs::write(&target, "secret\n").unwrap();
        let link = directory.path().join("standing.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let store = StandingStore::new(&link, std::sync::Arc::new(|_: &[StandingItem]| Ok(())));
        assert_eq!(store.entries().unwrap_err().code, ErrorCode::InvalidInput);
    }

    #[test]
    fn delete_removes_only_the_named_bullet() {
        let (_directory, store) = store();
        let first = store.add("keep", 50).unwrap();
        let second = store.add("remove", 50).unwrap();
        let deleted = store.delete(&second.memory.id, None).unwrap();
        assert_eq!(deleted.deleted.id, second.memory.id);
        let remaining = store.entries().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, first.memory.id);
    }
}
