//! The memory library: store, merge, search, and temporal reads.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use hearth_core::{
    Conversation, CoreError, Document, MemoryChange, MemoryUpdate, MemoryWrite, NewMemory, Save,
    Session,
};
use hearth_storage::rusqlite;
use hearth_storage::{Reader, StorageError};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::constants::{
    IMPORTANCE_WEIGHT, NEAR_DUPLICATE, PROMOTE_IMPORTANCE, RECENCY_HALF_LIFE_DAYS, RECENCY_WEIGHT,
    RRF_K, STALENESS_WEIGHT, TRIVIAL,
};
use crate::types::{
    MemoryError, MemoryItem, MemoryKind, MemoryStats, MemoryStatus, SourceKind, Standing,
    StandingMemory,
};

const MEMORY_SELECT: &str = "SELECT id, kind, content, tags, fingerprint, importance, status, \
     valid_from, valid_to, supersedes_id, source_kind, source_ref, standing, created_at, \
     last_used_at FROM memory_items";
const MEMORY_SELECT_M: &str = "SELECT m.id, m.kind, m.content, m.tags, m.fingerprint, \
     m.importance, m.status, m.valid_from, m.valid_to, m.supersedes_id, m.source_kind, \
     m.source_ref, m.standing, m.created_at, m.last_used_at FROM memory_items m";

const MEMORY_STATE_KIND: &str = "memory_state";
const MEMORY_STATE_TRANSPORT: &str = "system";
const MEMORY_STATE_KEY: &str = "memory";

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
    policy: crate::constants::MemoryPolicy,
}

impl MemoryService {
    /// Build the library over the permanent record with the default policy.
    pub fn new(session: Session) -> Self {
        Self::with_policy(session, crate::constants::MemoryPolicy::default())
    }

    /// Build the library with explicit budgets.
    pub fn with_policy(session: Session, policy: crate::constants::MemoryPolicy) -> Self {
        Self { session, policy }
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
        let standing = auto_standing(input.kind.as_str(), input.importance, Standing::None);
        let mut bump = standing != Standing::None;
        let mut writes = Vec::new();
        if let Some(old_id) = supersedes.as_deref() {
            let old = self
                .get(old_id)?
                .ok_or_else(|| MemoryError::NotFound(old_id.to_owned()))?;
            if old.standing != Standing::None {
                bump = true;
            }
            writes.push(MemoryWrite::new(
                MemoryChange::Update(MemoryUpdate {
                    id: old_id.to_owned(),
                    status: Some(MemoryStatus::Superseded.as_str().to_owned()),
                    valid_to: Some(now),
                    standing: Some(Standing::None.as_str().to_owned()),
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
            standing: standing.as_str().to_owned(),
            created_at: now,
            last_used_at: None,
        };
        writes.push(MemoryWrite::new(MemoryChange::Add(item.clone()), actor));
        self.commit(writes, bump)?;
        self.get(&item.id)?
            .ok_or_else(|| MemoryError::NotFound(item.id))
    }

    /// Apply a partial update.
    pub fn update(&self, id: &str, patch: MemoryPatch) -> Result<MemoryItem, MemoryError> {
        let existing = self
            .get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))?;
        let mut update = MemoryUpdate {
            id: id.to_owned(),
            ..MemoryUpdate::default()
        };
        let mut changed = false;
        let mut content_changed = false;
        let mut importance_changed = false;
        let mut status_changed = false;
        if let Some(content) = patch.content {
            let content = normalize(&content);
            if content.is_empty() {
                return Err(MemoryError::Invalid("memory content is empty".to_owned()));
            }
            reject_secret(&content)?;
            update.fingerprint = Some(fingerprint(&content));
            update.content = Some(content);
            content_changed = true;
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
            importance_changed = true;
            changed = true;
        }
        if let Some(status) = patch.status {
            update.status = Some(status.as_str().to_owned());
            status_changed = true;
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

        let new_importance = update.importance.unwrap_or(existing.importance);
        let new_status = update
            .status
            .as_deref()
            .unwrap_or(existing.status.as_str());
        let explicit = update.standing.as_deref().and_then(Standing::parse);
        let standing_after = if let Some(explicit) = explicit {
            explicit
        } else if new_status != MemoryStatus::Active.as_str() {
            Standing::None
        } else {
            auto_standing(existing.kind.as_str(), new_importance, existing.standing)
        };
        if update.standing.is_none() {
            update.standing = (standing_after != existing.standing)
                .then(|| standing_after.as_str().to_owned());
        }

        let mut bump = standing_after != existing.standing;
        if existing.standing != Standing::None && (content_changed || importance_changed) {
            bump = true;
        }
        if existing.standing != Standing::None && status_changed {
            bump = true;
        }
        if changed {
            self.commit(
                vec![MemoryWrite::new(MemoryChange::Update(update), "agent")],
                bump,
            )?;
        }
        self.get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))
    }

    /// Forget one memory, keeping a tombstone.
    pub fn forget(&self, id: &str) -> Result<MemoryItem, MemoryError> {
        let existing = self
            .get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))?;
        let bump = existing.standing != Standing::None;
        self.commit(
            vec![MemoryWrite::new(
                MemoryChange::Forget { id: id.to_owned() },
                "agent",
            )],
            bump,
        )?;
        self.get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))
    }

    /// Pin or unpin one memory.
    pub fn pin(&self, id: &str, pinned: bool) -> Result<MemoryItem, MemoryError> {
        let existing = self
            .get(id)?
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))?;
        let standing = if pinned {
            Standing::Pinned
        } else {
            auto_standing(existing.kind.as_str(), existing.importance, Standing::None)
        };
        let bump = existing.standing != standing;
        self.commit(
            vec![MemoryWrite::new(
                MemoryChange::Pin {
                    id: id.to_owned(),
                    standing: standing.as_str().to_owned(),
                },
                "agent",
            )],
            bump,
        )?;
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

    /// The current standing-memory block and its version.
    pub fn standing(&self) -> Result<StandingMemory, MemoryError> {
        let now = self.session.now_micros();
        let sql = format!(
            "{MEMORY_SELECT} WHERE status = 'active' \
             AND (valid_from IS NULL OR valid_from <= ?) \
             AND (valid_to IS NULL OR valid_to > ?) \
             AND (standing IN ('pinned','promoted') \
                  OR (importance >= {PROMOTE_IMPORTANCE} \
                      AND kind IN ('rule','preference','person'))) \
             ORDER BY CASE standing WHEN 'pinned' THEN 0 \
                      WHEN 'promoted' THEN 1 ELSE 2 END, \
                      importance DESC, COALESCE(last_used_at, 0) DESC, created_at"
        );
        let raw: Vec<RawMemory> = self.read(|reader| {
            reader.query(&sql, (now, now), |row| Ok(RawMemory::from_row(row)))
        })?;
        let mut candidates = Vec::new();
        for row in raw {
            candidates.push(row.into_item()?);
        }

        let cap = self.policy.standing_max_tokens;
        let mut items = Vec::new();
        let mut tokens = 0usize;
        let mut writes = Vec::new();
        let mut demoted = false;
        for mut item in candidates {
            let line_tokens = self.line_tokens(&item);
            let pinned = item.standing == Standing::Pinned;
            if pinned || tokens + line_tokens <= cap {
                tokens += line_tokens;
                if !pinned && item.standing == Standing::None {
                    item.standing = Standing::Promoted;
                    writes.push(MemoryWrite::new(
                        MemoryChange::Update(MemoryUpdate {
                            id: item.id.clone(),
                            standing: Some(Standing::Promoted.as_str().to_owned()),
                            ..MemoryUpdate::default()
                        }),
                        "system",
                    ));
                }
                items.push(item);
            } else if item.standing == Standing::Promoted {
                item.standing = Standing::None;
                writes.push(MemoryWrite::new(
                    MemoryChange::Update(MemoryUpdate {
                        id: item.id.clone(),
                        standing: Some(Standing::None.as_str().to_owned()),
                        ..MemoryUpdate::default()
                    }),
                    "system",
                ));
                demoted = true;
            }
        }
        if !writes.is_empty() {
            self.commit(writes, demoted)?;
        }
        let version = self.standing_version()?;
        Ok(StandingMemory { items, version })
    }

    /// The current standing-memory version.
    pub fn standing_version(&self) -> Result<u64, MemoryError> {
        let (_conversation, document) = self.memory_state()?;
        Ok(document
            .and_then(|doc| doc.json.get("standing_version").and_then(Value::as_u64))
            .unwrap_or(0))
    }

    /// Render one memory as a compact line.
    pub fn render_line(&self, item: &MemoryItem) -> String {
        format!(
            "[{}] {}, {}, p={}: {}",
            item.id,
            item.kind.as_str(),
            date_string(item.created_at),
            item.importance,
            item.content
        )
    }

    /// Render the standing block for the prompt.
    pub fn render_standing(&self, items: &[MemoryItem]) -> String {
        items
            .iter()
            .map(|item| self.render_line(item))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Render recalled memory for the prompt.
    pub fn render_recall(&self, items: &[MemoryItem]) -> String {
        self.render_standing(items)
    }

    /// Retrieve a small, diverse set of memories relevant to the input.
    pub fn recall(&self, query: &str) -> Result<Vec<MemoryItem>, MemoryError> {
        if trivial(query) {
            return Ok(Vec::new());
        }
        let now = self.session.now_micros();
        let fts = self.search(query, 40)?;
        let standing = self.standing()?.items;
        let tags = self.tag_matches(query)?;

        let mut scores: HashMap<String, (MemoryItem, f64)> = HashMap::new();
        for list in [&fts, &standing, &tags] {
            for (index, item) in list.iter().enumerate() {
                if !valid_at(item, now) {
                    continue;
                }
                let entry = scores
                    .entry(item.id.clone())
                    .or_insert_with(|| (item.clone(), 0.0));
                entry.1 += 1.0 / (RRF_K + (index + 1) as f64);
            }
        }

        let mut ranked: Vec<(MemoryItem, f64)> = scores
            .into_values()
            .map(|(item, rrf)| {
                let age_days = (now - item.created_at).max(0) as f64 / 86_400_000_000.0;
                let recency = 0.5f64.powf(age_days / RECENCY_HALF_LIFE_DAYS);
                let last_used = item.last_used_at.unwrap_or(item.created_at);
                let stale_days = (now - last_used).max(0) as f64 / 86_400_000_000.0;
                let staleness = 0.5f64.powf(stale_days / RECENCY_HALF_LIFE_DAYS);
                let score = rrf
                    + RECENCY_WEIGHT * recency
                    + IMPORTANCE_WEIGHT * (item.importance as f64 / 100.0)
                    - STALENESS_WEIGHT * staleness;
                (item, score)
            })
            .collect();
        ranked.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut selected: Vec<MemoryItem> = Vec::new();
        let mut kind_counts: BTreeMap<MemoryKind, usize> = BTreeMap::new();
        let mut tokens = 0usize;
        for (item, _score) in ranked {
            if selected.len() >= self.policy.recall_max_items {
                break;
            }
            if kind_counts.get(&item.kind).copied().unwrap_or(0) >= 2 {
                continue;
            }
            let item_tokens = tokens_of(&item.content);
            if selected
                .iter()
                .any(|chosen| jaccard(&tokens_of(&chosen.content), &item_tokens) >= NEAR_DUPLICATE)
            {
                continue;
            }
            let line_tokens = self.line_tokens(&item);
            if tokens + line_tokens > self.policy.recall_max_tokens {
                continue;
            }
            tokens += line_tokens;
            *kind_counts.entry(item.kind).or_default() += 1;
            selected.push(item);
        }

        if !selected.is_empty() {
            let writes: Vec<MemoryWrite> = selected
                .iter()
                .map(|item| {
                    MemoryWrite::new(
                        MemoryChange::Update(MemoryUpdate {
                            id: item.id.clone(),
                            last_used_at: Some(now),
                            ..MemoryUpdate::default()
                        }),
                        "session",
                    )
                })
                .collect();
            self.session.save_memory(writes)?;
            for item in &mut selected {
                item.last_used_at = Some(now);
            }
        }
        Ok(selected)
    }

    fn line_tokens(&self, item: &MemoryItem) -> usize {
        hearth_model::estimate_tokens(&self.render_line(item)).max(0) as usize
    }

    fn tag_matches(&self, query: &str) -> Result<Vec<MemoryItem>, MemoryError> {
        let tags: Vec<String> = tokens(query)
            .into_iter()
            .filter(|token| token.len() >= 3)
            .collect();
        if tags.is_empty() {
            return Ok(Vec::new());
        }
        let mut clauses = Vec::with_capacity(tags.len());
        let mut params: Vec<String> = Vec::with_capacity(tags.len());
        for tag in &tags {
            clauses.push("tags LIKE ?".to_owned());
            params.push(format!("%\"{tag}\"%"));
        }
        let sql = format!(
            "{MEMORY_SELECT} WHERE status = 'active' AND ({}) \
             ORDER BY importance DESC, created_at DESC LIMIT 20",
            clauses.join(" OR ")
        );
        let raw: Vec<RawMemory> = self.read(|reader| {
            let values: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|value| value as &dyn rusqlite::ToSql).collect();
            reader.query(
                &sql,
                rusqlite::params_from_iter(values),
                |row| Ok(RawMemory::from_row(row)),
            )
        })?;
        raw.into_iter().map(RawMemory::into_item).collect()
    }

    fn commit(&self, writes: Vec<MemoryWrite>, bump: bool) -> Result<(), MemoryError> {
        if !bump {
            self.session.save_memory(writes)?;
            return Ok(());
        }
        for attempt in 0..2 {
            let (conversation, document) = self.memory_state()?;
            let current = document
                .as_ref()
                .and_then(|doc| doc.json.get("standing_version"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let next = current + 1;
            let expected = document.as_ref().map(|doc| doc.version);
            let mut save = Save::new(&conversation.id);
            for write in &writes {
                save = save.memory(write.clone());
            }
            save = save.document(
                MEMORY_STATE_KIND,
                expected,
                json!({"standing_version": next}),
            );
            match self.session.save(save) {
                Ok(_) => return Ok(()),
                Err(CoreError::DocumentVersion { .. }) if attempt == 0 => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(MemoryError::Invalid(
            "memory state version changed twice".to_owned(),
        ))
    }

    fn memory_state(&self) -> Result<(Conversation, Option<Document>), MemoryError> {
        let conversation = match self.session.conversation_by_transport(
            MEMORY_STATE_TRANSPORT,
            MEMORY_STATE_KEY,
        )? {
            Some(conversation) => conversation,
            None => self.session.create_conversation(
                "Memory state",
                MEMORY_STATE_TRANSPORT,
                MEMORY_STATE_KEY,
            )?,
        };
        let document = self.session.document(&conversation.id, MEMORY_STATE_KIND)?;
        Ok((conversation, document))
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
        let merged_importance = existing.importance.max(importance);
        let standing = auto_standing(existing.kind.as_str(), merged_importance, existing.standing);
        let mut bump = existing.standing != standing;
        if existing.standing != Standing::None && merged_importance != existing.importance {
            bump = true;
        }
        let update = MemoryUpdate {
            id: existing.id.clone(),
            tags: Some(merged_tags),
            importance: Some(merged_importance),
            standing: (standing != existing.standing).then(|| standing.as_str().to_owned()),
            ..MemoryUpdate::default()
        };
        self.commit(
            vec![MemoryWrite::new(MemoryChange::Update(update), actor)],
            bump,
        )?;
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

fn auto_standing(kind: &str, importance: i64, current: Standing) -> Standing {
    if current == Standing::Pinned {
        return Standing::Pinned;
    }
    if importance >= PROMOTE_IMPORTANCE
        && matches!(kind, "rule" | "preference" | "person")
    {
        Standing::Promoted
    } else {
        Standing::None
    }
}

fn trivial(query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.chars().count() <= 2 {
        return true;
    }
    if TRIVIAL.contains(&trimmed.to_lowercase().as_str()) {
        return true;
    }
    !trimmed.chars().any(|character| character.is_alphanumeric())
}

fn valid_at(item: &MemoryItem, now: i64) -> bool {
    item.status == MemoryStatus::Active
        && item.created_at <= now
        && item.valid_from.is_none_or(|from| from <= now)
        && item.valid_to.is_none_or(|to| to > now)
}

fn date_string(micros: i64) -> String {
    let seconds = micros.div_euclid(1_000_000);
    let date = OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn tokens_of(content: &str) -> BTreeSet<String> {
    tokens(content)
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
