//! The memory library: store, merge, search, and temporal reads.

use std::collections::BTreeSet;

use hearth_core::{MemoryChange, MemoryUpdate, MemoryWrite, NewMemory, Session};
use hearth_storage::rusqlite;
use hearth_storage::{Reader, StorageError};
use uuid::Uuid;

use crate::types::{
    MemoryError, MemoryItem, MemoryKind, MemoryStats, MemoryStatus, SourceKind, Standing,
};

const MEMORY_SELECT: &str = "SELECT id, kind, content, tags, fingerprint, importance, status, \
     valid_from, valid_to, supersedes_id, source_kind, source_ref, standing, created_at, \
     last_used_at FROM memory_items";
const MEMORY_SELECT_M: &str = "SELECT m.id, m.kind, m.content, m.tags, m.fingerprint, \
     m.importance, m.status, m.valid_from, m.valid_to, m.supersedes_id, m.source_kind, \
     m.source_ref, m.standing, m.created_at, m.last_used_at FROM memory_items m";

const NEAR_DUPLICATE: f64 = 0.85;

/// Input for one new memory.
#[derive(Debug, Clone)]
pub struct NewMemoryInput {
    pub content: String,
    pub kind: MemoryKind,
    pub tags: Vec<String>,
    pub importance: i64,
    pub source_kind: SourceKind,
    pub source_ref: Option<String>,
    pub supersedes: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_to: Option<i64>,
}

/// A partial update to one memory.
#[derive(Debug, Clone, Default)]
pub struct MemoryPatch {
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub importance: Option<i64>,
    pub status: Option<MemoryStatus>,
    pub valid_to: Option<i64>,
    pub last_used_at: Option<i64>,
    pub standing: Option<Standing>,
}

/// The memory library for one home.
#[derive(Clone)]
pub struct MemoryService {
    session: Session,
}

impl MemoryService {
    /// Build the library over the permanent record.
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Save one memory. Exact and near duplicates merge into the existing row.
    pub fn save(&self, input: NewMemoryInput) -> Result<MemoryItem, MemoryError> {
        self.save_with_actor(input, "agent")
    }

    /// Save one memory with an explicit audit actor.
    pub fn save_with_actor(
        &self,
        input: NewMemoryInput,
        actor: &str,
    ) -> Result<MemoryItem, MemoryError> {
        let content = normalize(&input.content);
        if content.is_empty() {
            return Err(MemoryError::Invalid("memory content is empty".to_owned()));
        }
        reject_secret(&content)?;
        if !(0..=100).contains(&input.importance) {
            return Err(MemoryError::Invalid(
                "importance must be between 0 and 100".to_owned(),
            ));
        }
        let tags = normalize_tags(input.tags);
        let fingerprint = fingerprint(&content);

        if let Some(existing) = self.find_by_fingerprint(&fingerprint)? {
            return self.merge(&existing, &tags, input.importance, actor);
        }
        if let Some(existing) = self.find_near(&content)? {
            return self.merge(&existing, &tags, input.importance, actor);
        }

        let now = self.session.now_micros();
        let supersedes = input.supersedes;
        let mut writes = Vec::new();
        if let Some(old_id) = supersedes.as_deref() {
            if self.get(old_id)?.is_none() {
                return Err(MemoryError::NotFound(old_id.to_owned()));
            }
            writes.push(MemoryWrite::new(
                MemoryChange::Update(MemoryUpdate {
                    id: old_id.to_owned(),
                    status: Some(MemoryStatus::Superseded.as_str().to_owned()),
                    valid_to: Some(now),
                    ..MemoryUpdate::default()
                }),
                actor,
            ));
        }
        let item = NewMemory {
            id: self.session.new_id(),
            kind: input.kind.as_str().to_owned(),
            content: content.clone(),
            tags,
            fingerprint,
            importance: input.importance,
            status: MemoryStatus::Active.as_str().to_owned(),
            valid_from: input.valid_from,
            valid_to: input.valid_to,
            supersedes_id: supersedes,
            source_kind: input.source_kind.as_str().to_owned(),
            source_ref: input.source_ref,
            standing: Standing::None.as_str().to_owned(),
            created_at: now,
            last_used_at: None,
        };
        writes.push(MemoryWrite::new(MemoryChange::Add(item.clone()), actor));
        self.session.save_memory(writes)?;
        self.get(&item.id)?
            .ok_or_else(|| MemoryError::NotFound(item.id))
    }

    /// Apply a partial update.
    pub fn update(&self, id: &str, patch: MemoryPatch) -> Result<MemoryItem, MemoryError> {
        if self.get(id)?.is_none() {
            return Err(MemoryError::NotFound(id.to_owned()));
        }
        let mut update = MemoryUpdate {
            id: id.to_owned(),
            ..MemoryUpdate::default()
        };
        let mut changed = false;
        if let Some(content) = patch.content {
            let content = normalize(&content);
            if content.is_empty() {
                return Err(MemoryError::Invalid("memory content is empty".to_owned()));
            }
            reject_secret(&content)?;
            update.fingerprint = Some(fingerprint(&content));
            update.content = Some(content);
            changed = true;
        }
        if let Some(tags) = patch.tags {
            update.tags = Some(normalize_tags(tags));
            changed = true;
        }
        if let Some(importance) = patch.importance {
            if !(0..=100).contains(&importance) {
                return Err(MemoryError::Invalid(
                    "importance must be between 0 and 100".to_owned(),
                ));
            }
            update.importance = Some(importance);
            changed = true;
        }
        if let Some(status) = patch.status {
            update.status = Some(status.as_str().to_owned());
            changed = true;
        }
        if let Some(valid_to) = patch.valid_to {
            update.valid_to = Some(valid_to);
            changed = true;
        }
        if let Some(last_used_at) = patch.last_used_at {
            update.last_used_at = Some(last_used_at);
            changed = true;
        }
        if let Some(standing) = patch.standing {
            update.standing = Some(standing.as_str().to_owned());
            changed = true;
        }
        if changed {
            self.session.save_memory(vec![MemoryWrite::new(
                MemoryChange::Update(update),
                "agent",
            )])?;
        }
        self.get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))
    }

    /// Forget one memory, keeping a tombstone.
    pub fn forget(&self, id: &str) -> Result<MemoryItem, MemoryError> {
        if self.get(id)?.is_none() {
            return Err(MemoryError::NotFound(id.to_owned()));
        }
        self.session.save_memory(vec![MemoryWrite::new(
            MemoryChange::Forget { id: id.to_owned() },
            "agent",
        )])?;
        self.get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))
    }

    /// Pin or unpin one memory.
    pub fn pin(&self, id: &str, pinned: bool) -> Result<MemoryItem, MemoryError> {
        if self.get(id)?.is_none() {
            return Err(MemoryError::NotFound(id.to_owned()));
        }
        let standing = if pinned {
            Standing::Pinned
        } else {
            Standing::None
        };
        self.session.save_memory(vec![MemoryWrite::new(
            MemoryChange::Pin {
                id: id.to_owned(),
                standing: standing.as_str().to_owned(),
            },
            "agent",
        )])?;
        self.get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))
    }

    /// Read one memory by id, including tombstones.
    pub fn get(&self, id: &str) -> Result<Option<MemoryItem>, MemoryError> {
        let sql = format!("{MEMORY_SELECT} WHERE id = ?");
        let raw: Option<RawMemory> = self.read(|reader| {
            reader.query_optional(&sql, (id,), |row| Ok(RawMemory::from_row(row)))
        })?;
        raw.map(RawMemory::into_item).transpose()
    }

    /// Search active memories by full text and tags.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryItem>, MemoryError> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 100) as i64;
        let sql = format!(
            "{MEMORY_SELECT_M} JOIN memory_fts f ON m.rowid = f.rowid \
             WHERE memory_fts MATCH ? AND m.status = 'active' \
             ORDER BY bm25(memory_fts) LIMIT ?"
        );
        let raw: Vec<RawMemory> = self.read(|reader| {
            reader.query(&sql, (fts.as_str(), limit), |row| {
                Ok(RawMemory::from_row(row))
            })
        })?;
        raw.into_iter().map(RawMemory::into_item).collect()
    }

    /// Count memories by status and kind.
    pub fn stats(&self) -> Result<MemoryStats, MemoryError> {
        let rows: Vec<(String, String, i64)> = self.read(|reader| {
            reader.query(
                "SELECT status, kind, COUNT(*) FROM memory_items GROUP BY status, kind",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })?;
        let mut stats = MemoryStats::default();
        for (status, kind, count) in rows {
            stats.total += count;
            *stats.by_status.entry(status).or_default() += count;
            *stats.by_kind.entry(kind).or_default() += count;
        }
        Ok(stats)
    }

    /// Read the memories that were true at one instant.
    pub fn as_of(
        &self,
        query: Option<&str>,
        at_micros: i64,
    ) -> Result<Vec<MemoryItem>, MemoryError> {
        let fts = query.map(fts_query).unwrap_or_default();
        let raw: Vec<RawMemory> = if fts.is_empty() {
            let sql = format!(
                "{MEMORY_SELECT} WHERE status != 'deleted' AND created_at <= ? \
                 AND (valid_from IS NULL OR valid_from <= ?) \
                 AND (valid_to IS NULL OR valid_to > ?) \
                 ORDER BY created_at LIMIT 50"
            );
            self.read(|reader| {
                reader.query(&sql, (at_micros, at_micros, at_micros), |row| {
                    Ok(RawMemory::from_row(row))
                })
            })?
        } else {
            let sql = format!(
                "{MEMORY_SELECT_M} JOIN memory_fts f ON m.rowid = f.rowid \
                 WHERE memory_fts MATCH ? AND m.status != 'deleted' AND m.created_at <= ? \
                 AND (m.valid_from IS NULL OR m.valid_from <= ?) \
                 AND (m.valid_to IS NULL OR m.valid_to > ?) \
                 ORDER BY m.created_at LIMIT 50"
            );
            self.read(|reader| {
                reader.query(
                    &sql,
                    (fts.as_str(), at_micros, at_micros, at_micros),
                    |row| Ok(RawMemory::from_row(row)),
                )
            })?
        };
        raw.into_iter().map(RawMemory::into_item).collect()
    }

    fn find_by_fingerprint(&self, fingerprint: &str) -> Result<Option<MemoryItem>, MemoryError> {
        let sql = format!("{MEMORY_SELECT} WHERE fingerprint = ? AND status = 'active' LIMIT 1");
        let raw: Option<RawMemory> = self.read(|reader| {
            reader.query_optional(&sql, (fingerprint,), |row| Ok(RawMemory::from_row(row)))
        })?;
        raw.map(RawMemory::into_item).transpose()
    }

    fn find_near(&self, content: &str) -> Result<Option<MemoryItem>, MemoryError> {
        let candidate_tokens = tokens(content);
        if candidate_tokens.is_empty() {
            return Ok(None);
        }
        let sql = format!("{MEMORY_SELECT} WHERE status = 'active' ORDER BY created_at LIMIT 500");
        let raw: Vec<RawMemory> =
            self.read(|reader| reader.query(&sql, [], |row| Ok(RawMemory::from_row(row))))?;
        for row in raw {
            let existing = tokens(&row.content);
            if jaccard(&candidate_tokens, &existing) >= NEAR_DUPLICATE {
                return row.into_item().map(Some);
            }
        }
        Ok(None)
    }

    fn merge(
        &self,
        existing: &MemoryItem,
        tags: &[String],
        importance: i64,
        actor: &str,
    ) -> Result<MemoryItem, MemoryError> {
        let mut merged_tags = existing.tags.clone();
        for tag in tags {
            if !merged_tags.contains(tag) {
                merged_tags.push(tag.clone());
            }
        }
        let update = MemoryUpdate {
            id: existing.id.clone(),
            tags: Some(merged_tags),
            importance: Some(existing.importance.max(importance)),
            ..MemoryUpdate::default()
        };
        self.session
            .save_memory(vec![MemoryWrite::new(MemoryChange::Update(update), actor)])?;
        self.get(&existing.id)?
            .ok_or_else(|| MemoryError::NotFound(existing.id.clone()))
    }

    fn read<T>(
        &self,
        run: impl FnOnce(&Reader<'_>) -> Result<T, StorageError>,
    ) -> Result<T, MemoryError> {
        Ok(self.session.storage().read(run)?)
    }
}

struct RawMemory {
    id: String,
    kind: String,
    content: String,
    tags: String,
    fingerprint: String,
    importance: i64,
    status: String,
    valid_from: Option<i64>,
    valid_to: Option<i64>,
    supersedes_id: Option<String>,
    source_kind: String,
    source_ref: Option<String>,
    standing: String,
    created_at: i64,
    last_used_at: Option<i64>,
}

impl RawMemory {
    fn from_row(row: &rusqlite::Row<'_>) -> Self {
        Self {
            id: row.get(0).unwrap_or_default(),
            kind: row.get(1).unwrap_or_default(),
            content: row.get(2).unwrap_or_default(),
            tags: row.get(3).unwrap_or_default(),
            fingerprint: row.get(4).unwrap_or_default(),
            importance: row.get(5).unwrap_or_default(),
            status: row.get(6).unwrap_or_default(),
            valid_from: row.get(7).unwrap_or(None),
            valid_to: row.get(8).unwrap_or(None),
            supersedes_id: row.get(9).unwrap_or(None),
            source_kind: row.get(10).unwrap_or_default(),
            source_ref: row.get(11).unwrap_or(None),
            standing: row.get(12).unwrap_or_default(),
            created_at: row.get(13).unwrap_or_default(),
            last_used_at: row.get(14).unwrap_or(None),
        }
    }

    fn into_item(self) -> Result<MemoryItem, MemoryError> {
        let tags: Vec<String> = serde_json::from_str(&self.tags)
            .map_err(|error| MemoryError::Corrupt(format!("memory tags: {error}")))?;
        let kind = MemoryKind::parse(&self.kind)
            .ok_or_else(|| MemoryError::Corrupt(format!("unknown memory kind: {}", self.kind)))?;
        let status = MemoryStatus::parse(&self.status).ok_or_else(|| {
            MemoryError::Corrupt(format!("unknown memory status: {}", self.status))
        })?;
        let source_kind = SourceKind::parse(&self.source_kind).ok_or_else(|| {
            MemoryError::Corrupt(format!("unknown memory source: {}", self.source_kind))
        })?;
        let standing = Standing::parse(&self.standing).ok_or_else(|| {
            MemoryError::Corrupt(format!("unknown memory standing: {}", self.standing))
        })?;
        Ok(MemoryItem {
            id: self.id,
            kind,
            content: self.content,
            tags,
            fingerprint: self.fingerprint,
            importance: self.importance,
            status,
            valid_from: self.valid_from,
            valid_to: self.valid_to,
            supersedes_id: self.supersedes_id,
            source_kind,
            source_ref: self.source_ref,
            standing,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
        })
    }
}

fn normalize(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut unique = BTreeSet::new();
    for tag in tags {
        let tag = tag.trim().to_lowercase();
        if !tag.is_empty() {
            unique.insert(tag);
        }
    }
    unique.into_iter().collect()
}

fn fingerprint(content: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        normalize(content).to_lowercase().as_bytes(),
    )
    .to_string()
}

fn tokens(content: &str) -> BTreeSet<String> {
    content
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.len() >= 2)
        .map(str::to_owned)
        .collect()
}

fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count() as f64;
    let union = left.union(right).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn fts_query(query: &str) -> String {
    tokens(query)
        .into_iter()
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn reject_secret(content: &str) -> Result<(), MemoryError> {
    let lower = content.to_ascii_lowercase();
    for prefix in ["sk-", "ghp_", "xoxb-", "akia"] {
        if lower.contains(prefix) {
            return Err(MemoryError::Secret(
                "credential-like content was refused".to_owned(),
            ));
        }
    }
    for token in content.split_whitespace() {
        if let Some((left, right)) = token.split_once(':')
            && left.len() >= 8
            && left.chars().all(|character| character.is_ascii_digit())
            && right.len() >= 20
        {
            return Err(MemoryError::Secret(
                "credential-like content was refused".to_owned(),
            ));
        }
        let lower_token = token.to_ascii_lowercase();
        for key in ["api_key", "apikey", "token", "password", "secret"] {
            if let Some(rest) = lower_token.strip_prefix(key) {
                let rest = rest.trim_start_matches(['=', ':']);
                if rest.len() >= 8 && !rest.starts_with('$') {
                    return Err(MemoryError::Secret(
                        "credential-like content was refused".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(())
}
