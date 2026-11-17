//! Hearth memory.
//!
//! Memory has three jobs and each gets its own shape:
//!
//! * **Rules** are how the user wants things done. They are small, they live in
//!   the cached prompt prefix, and they are never retrieved because they are
//!   always present.
//! * **Knowledge** is what the user said. It is one table with one index,
//!   reachable by subject, by keyword, and by meaning, and it is shown as a
//!   compact digest that says when it is showing a subset.
//! * **Evidence** is the transcript. It is never edited, and it is read only when
//!   the model asks for it.
//!
//! The design rule that keeps the system honest: **no memory operation calls a
//! model.** Writing, reconciling, ranking, and rendering are deterministic. The
//! agent already is the model; it decides what to remember through a tool call,
//! and it decides what to replace by naming the row it replaces. Every token of
//! memory value therefore comes with no extra request and no extra latency.
//!
//! Recall itself is four stages, none of them generative: three cheap channels
//! produce a wide pool, reciprocal rank fuses them, a linear score over cheap
//! features ranks the pool, and a greedy fill spends the token budget on the best
//! value per token.

mod consolidate;
mod digest;
mod embedding;
mod errors;
mod extract;
mod model;
mod query;
mod rank;
mod retrieval;
mod store;
mod text;
mod tools;
mod transcript;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use hearth_core::{Session, Zone};

pub use consolidate::{
    CanonicalGroup, CompoundRow, ConsolidationReport, NearPair, SubjectConflict,
};
pub use digest::Coverage;
pub use embedding::{
    BekkoEmbedding, EmbeddingProvider, MODEL_DIMS, MODEL_NAME, download_model, model_files_present,
};
pub use errors::{Code, MemoryError, Result};
pub use extract::{Candidate, candidates};
pub use model::{Kind, Memory, NewMemory, Source, WriteOutcome};
pub use store::{Breakdown, CONTENT_LIMIT, breakdown_on, fingerprint};
pub use tools::{FORGET, RECALL, SAVE, TRANSCRIPT, ToolMemory};
pub use transcript::{EntryHit, TranscriptRead};

/// How much of the prompt memory may occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryConfig {
    /// The budget for rules in the cached prefix.
    pub rules_max_tokens: u64,
    /// The most statements one digest may show.
    pub recall_max_items: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            rules_max_tokens: 800,
            recall_max_items: 10,
        }
    }
}

impl MemoryConfig {
    pub fn bounded(self) -> Self {
        Self {
            rules_max_tokens: self.rules_max_tokens.clamp(50, 4_000),
            recall_max_items: self.recall_max_items.clamp(1, 64),
        }
    }
}

/// What one recall produced.
#[derive(Debug, Clone, Default)]
pub struct Recall {
    /// The statements chosen for the digest, best first.
    pub items: Vec<Memory>,
    /// How much more the store holds about what the message named.
    pub coverage: Coverage,
    /// The rendered block, or an empty string when nothing was recalled.
    pub digest: String,
}

/// The rendered rules and their version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub text: String,
    pub version: u64,
    pub count: usize,
}

/// The cached vector index for one model.
struct VectorCache {
    model: String,
    dims: usize,
    rows: Vec<store::VectorRow>,
}

/// Memory as the runtime sees it.
///
/// Cloning is cheap and shares every internal cache, which is what lets the tool
/// set hold the same service the agent loop reads from.
#[derive(Clone)]
pub struct MemoryService {
    inner: Arc<Inner>,
}

struct Inner {
    session: Session,
    config: RwLock<MemoryConfig>,
    embedder: RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    configured: AtomicBool,
    vectors: Mutex<Option<VectorCache>>,
    subjects: Mutex<Option<Vec<String>>>,
}

impl std::fmt::Debug for MemoryService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryService")
            .field("config", &self.config())
            .field("semantic", &self.embeddings_active())
            .finish_non_exhaustive()
    }
}

impl MemoryService {
    /// Attach memory to a session's database.
    ///
    /// There is no separate memory file: a memory, its provenance, and the entry
    /// that produced it share one writer, one transaction boundary, and one
    /// backup.
    pub fn new(session: Session) -> Self {
        Self {
            inner: Arc::new(Inner {
                session,
                config: RwLock::new(MemoryConfig::default()),
                embedder: RwLock::new(None),
                configured: AtomicBool::new(false),
                vectors: Mutex::new(None),
                subjects: Mutex::new(None),
            }),
        }
    }

    pub fn with_config(self, config: MemoryConfig) -> Self {
        self.configure(config);
        self
    }

    /// Apply a new budget without rebuilding the service.
    pub fn configure(&self, config: MemoryConfig) {
        if let Ok(mut slot) = self.inner.config.write() {
            *slot = config.bounded();
        }
    }

    /// Record whether semantic recall is configured at all. The backend reports
    /// separately whether its model is actually loaded.
    pub fn with_embeddings(self, configured: bool) -> Self {
        self.inner.configured.store(configured, Ordering::Relaxed);
        self
    }

    /// Attach a ready embedder.
    pub fn with_embedder(self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        self.activate_embedder(embedder);
        self
    }

    /// Attach an embedder after startup.
    pub fn activate_embedder(&self, embedder: Arc<dyn EmbeddingProvider>) {
        self.inner.configured.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = self.inner.embedder.write() {
            *slot = Some(embedder);
        }
        self.invalidate_vectors();
    }

    pub fn config(&self) -> MemoryConfig {
        self.inner
            .config
            .read()
            .map(|config| *config)
            .unwrap_or_default()
    }

    pub fn session(&self) -> &Session {
        &self.inner.session
    }

    /// `hybrid` when a local model is loaded, otherwise `lexical`.
    pub fn mode(&self) -> &'static str {
        if self.embeddings_active() {
            "hybrid"
        } else {
            "lexical"
        }
    }

    pub fn embeddings_configured(&self) -> bool {
        self.inner.configured.load(Ordering::Relaxed)
    }

    pub fn embeddings_active(&self) -> bool {
        self.inner
            .embedder
            .read()
            .map(|slot| slot.is_some())
            .unwrap_or(false)
    }

    pub fn embedding_model(&self) -> Option<String> {
        self.inner
            .embedder
            .read()
            .ok()
            .and_then(|slot| slot.as_ref().map(|model| model.model().to_owned()))
    }

    /// The number of current memories.
    pub fn count(&self) -> Result<usize> {
        store::count(&self.inner.session)
    }

    /// The pinned, active, and retired counts.
    pub fn breakdown(&self) -> Result<Breakdown> {
        store::breakdown(&self.inner.session)
    }

    /// Audit the store and close its canonical duplicates.
    pub fn consolidate(&self) -> Result<ConsolidationReport> {
        consolidate::run(&self.inner.session)
    }

    /// The rendered rules body and a hash of it.
    ///
    /// The prompt prefix hashes the text itself, so the version is for the audit
    /// document and the status report: it changes exactly when the rendered block
    /// changes, which is what an operator watching for a rule edit wants.
    pub fn rules(&self) -> Result<Rules> {
        let rules = store::rules(&self.inner.session)?;
        let text = digest::rules_body(&rules, self.config().rules_max_tokens);
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        Ok(Rules {
            version: hasher.finish(),
            count: rules.len(),
            text,
        })
    }

    /// Recall what this message needs.
    pub fn recall(&self, text: &str) -> Result<Recall> {
        self.recall_at(text, None)
    }

    /// Recall as of an instant, for a question about what used to be true.
    pub fn recall_at(&self, text: &str, at: Option<i64>) -> Result<Recall> {
        let known = self.known_subjects()?;
        let parsed = query::analyze(text, &known);
        if parsed.trivial {
            return Ok(Recall::default());
        }
        let now = self.inner.session.now_micros();
        let mut memories: HashMap<String, Memory> = HashMap::new();
        let mut channels: Vec<Vec<String>> = Vec::new();

        if let Some(expression) = retrieval::match_all(&parsed.terms) {
            let items = store::lexical(
                &self.inner.session,
                &expression,
                at,
                None,
                rank::POOL as i64,
            )?;
            channels.push(absorb(items, &mut memories));
        }
        if let Some(expression) =
            retrieval::match_any(&parsed.terms, &parsed.expanded, &parsed.entities)
        {
            let items = store::lexical(
                &self.inner.session,
                &expression,
                at,
                None,
                rank::POOL as i64,
            )?;
            channels.push(absorb(items, &mut memories));
        }
        for entity in parsed.entities.iter().take(4) {
            let mut found = store::by_subject(&self.inner.session, entity, at, None, 32)?;
            found.extend(store::by_subject_prefix(
                &self.inner.session,
                entity,
                at,
                32,
            )?);
            channels.push(absorb(found, &mut memories));
        }
        // The vector index carries only current statements, so it cannot answer
        // a question about a past instant. An as-of read stays lexical and
        // entity-based rather than being polluted with what is true today.
        if at.is_none()
            && let Some(ids) = self.semantic_channel(&parsed.raw)
        {
            let items = store::by_ids(&self.inner.session, &ids)?;
            channels.push(absorb(items, &mut memories));
        }

        let entities: HashSet<String> = parsed.entities.iter().cloned().collect();
        let terms: Vec<String> = parsed.terms.iter().map(|term| term.text.clone()).collect();
        let fused = rank::fuse(&channels);
        let ranked = rank::rank(&fused, &memories, &entities, &terms, now);
        let diverse = rank::diversify(ranked, &entities);
        let config = self.config();
        let coverage = self.coverage(&parsed.entities, at)?;
        let items = rank::fill(diverse, config.recall_max_items as usize);
        let digest = digest::recall_body(&items, &coverage, now);
        Ok(Recall {
            items,
            coverage,
            digest,
        })
    }

    /// How much the store holds about each named subject.
    fn coverage(&self, entities: &[String], at: Option<i64>) -> Result<Coverage> {
        let mut coverage = Coverage::default();
        for entity in entities.iter().take(3) {
            let current = store::subject_count(&self.inner.session, entity, at)?;
            if current == 0 {
                continue;
            }
            coverage.current.insert(entity.clone(), current);
            let earlier = store::historical_count(&self.inner.session, entity)?;
            if earlier > 0 {
                coverage.earlier.insert(entity.clone(), earlier);
            }
        }
        Ok(coverage)
    }

    /// The vector channel, when a model is loaded.
    fn semantic_channel(&self, text: &str) -> Option<Vec<String>> {
        let embedder = self.inner.embedder.read().ok()?.clone()?;
        let values = embedder.embed(text).ok()?;
        if values.len() != embedder.dims() {
            return None;
        }
        let query = retrieval::quantise(&values);
        let model = embedder.model().to_owned();
        let dims = embedder.dims();
        let mut cache = self.inner.vectors.lock().ok()?;
        let reload = cache
            .as_ref()
            .is_none_or(|cached| cached.model != model || cached.dims != dims);
        if reload {
            *cache = Some(VectorCache {
                model,
                dims,
                rows: store::vectors(&self.inner.session, embedder.model(), embedder.dims())
                    .ok()?,
            });
        }
        cache
            .as_ref()
            .map(|cached| retrieval::top_vectors(&cached.rows, &query, 24))
    }

    /// Write one statement.
    pub fn write(&self, item: NewMemory) -> Result<WriteOutcome> {
        store::validate(&item)?;
        let vector = self.embed_for_write(&item);
        let supersedes = item.supersedes.clone();
        let outcome = store::insert(
            &self.inner.session,
            store::Pending {
                item,
                vector: vector.clone(),
            },
            self.config().rules_max_tokens,
        )?;
        if outcome.created() {
            self.remember(&outcome, vector.as_ref(), supersedes.as_deref());
        }
        Ok(outcome)
    }

    /// Write several statements in one transaction.
    ///
    /// This is the import and benchmark path. A conversation turn writes one
    /// statement at a time, so the single-writer path stays the common one.
    pub fn write_many(&self, items: Vec<NewMemory>) -> Result<Vec<WriteOutcome>> {
        for item in &items {
            store::validate(item)?;
        }
        let mut kept = Vec::with_capacity(items.len());
        let pending = items
            .into_iter()
            .map(|item| {
                let vector = self.embed_for_write(&item);
                kept.push((vector.clone(), item.supersedes.clone()));
                store::Pending { item, vector }
            })
            .collect();
        let outcomes =
            store::insert_many(&self.inner.session, pending, self.config().rules_max_tokens)?;
        for (outcome, (vector, supersedes)) in outcomes.iter().zip(kept) {
            if outcome.created() {
                self.remember(outcome, vector.as_ref(), supersedes.as_deref());
            }
        }
        Ok(outcomes)
    }

    /// Remove one statement permanently.
    pub fn forget(&self, id: &str) -> Result<()> {
        store::forget(&self.inner.session, id)?;
        if let Ok(mut cache) = self.inner.vectors.lock()
            && let Some(cache) = cache.as_mut()
        {
            cache.rows.retain(|row| row.memory_id != id);
        }
        Ok(())
    }

    /// One statement by id.
    pub fn get(&self, id: &str) -> Result<Option<Memory>> {
        store::get(&self.inner.session, id)
    }

    /// Everything known about a subject.
    pub fn subject(&self, subject: &str, at: Option<i64>, limit: i64) -> Result<Vec<Memory>> {
        self.subject_of_kind(subject, at, None, limit)
    }

    /// Everything about a subject, optionally narrowed to one kind.
    pub fn subject_of_kind(
        &self,
        subject: &str,
        at: Option<i64>,
        kind: Option<Kind>,
        limit: i64,
    ) -> Result<Vec<Memory>> {
        let normalized = query::normalize(subject);
        store::by_subject(&self.inner.session, &normalized, at, kind, limit)
    }

    /// Recent statements when a caller asks for memory without a topic.
    pub fn recent(&self, at: Option<i64>, limit: i64) -> Result<Vec<Memory>> {
        self.recent_of_kind(at, None, limit)
    }

    /// Recent statements, optionally narrowed to one kind.
    pub fn recent_of_kind(
        &self,
        at: Option<i64>,
        kind: Option<Kind>,
        limit: i64,
    ) -> Result<Vec<Memory>> {
        store::recent(&self.inner.session, at, kind, limit)
    }

    /// A keyword, subject, or entity search, for the tool.
    pub fn search(&self, text: &str, at: Option<i64>, limit: i64) -> Result<Vec<Memory>> {
        self.search_of_kind(text, at, None, limit)
    }

    /// A keyword, subject, or entity search narrowed to one kind.
    pub fn search_of_kind(
        &self,
        text: &str,
        at: Option<i64>,
        kind: Option<Kind>,
        limit: i64,
    ) -> Result<Vec<Memory>> {
        let known = self.known_subjects()?;
        let parsed = query::analyze(text, &known);
        if parsed.trivial {
            return store::recent(&self.inner.session, at, kind, limit);
        }
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for expression in [
            retrieval::match_all(&parsed.terms),
            retrieval::match_any(&parsed.terms, &parsed.expanded, &parsed.entities),
        ]
        .into_iter()
        .flatten()
        {
            for memory in store::lexical(&self.inner.session, &expression, at, kind, limit)? {
                if seen.insert(memory.id.clone()) {
                    hits.push(memory);
                }
            }
        }
        for entity in parsed.entities.iter().take(3) {
            for memory in store::by_subject(&self.inner.session, entity, at, kind, limit)? {
                if seen.insert(memory.id.clone()) {
                    hits.push(memory);
                }
            }
        }
        if at.is_none()
            && hits.len() < limit.max(1) as usize
            && let Some(ids) = self.semantic_channel(text)
        {
            for memory in store::by_ids(&self.inner.session, &ids)? {
                if kind.is_some_and(|kind| memory.kind != kind) {
                    continue;
                }
                if seen.insert(memory.id.clone()) {
                    hits.push(memory);
                }
            }
        }
        hits.truncate(limit.max(1) as usize);
        Ok(hits)
    }

    /// Search the transcript.
    pub fn transcript(&self, text: &str, limit: i64) -> Result<Vec<EntryHit>> {
        transcript::search(&self.inner.session, text, limit)
    }

    /// Read one transcript entry by `conversation#seq`.
    pub fn entry(&self, reference: &str) -> Result<Option<TranscriptRead>> {
        transcript::read(&self.inner.session, reference)
    }

    /// Index memories that have no vector for the active model.
    ///
    /// This runs off the reply path. A write can embed inline because it already
    /// has the text; a backfill happens after a model change or a fresh install.
    pub fn index_pending_vectors(&self) -> Result<usize> {
        let Some(embedder) = self.embedder() else {
            return Ok(0);
        };
        let model = embedder.model().to_owned();
        let dims = embedder.dims();
        let mut indexed = 0;
        'backfill: loop {
            let pending = store::pending_vectors(&self.inner.session, &model, dims, 256)?;
            if pending.is_empty() {
                break;
            }
            let mut written = 0;
            for memory in pending {
                match embedder.embed(&memory.content) {
                    Ok(values) => {
                        let vector = store::VectorWrite {
                            model: model.clone(),
                            dims,
                            values,
                        };
                        if store::set_vector(&self.inner.session, &memory.id, &vector).is_ok() {
                            indexed += 1;
                            written += 1;
                        }
                    }
                    Err(_) => break 'backfill,
                }
            }
            if written == 0 {
                break;
            }
        }
        if indexed > 0 {
            self.invalidate_vectors();
        }
        Ok(indexed)
    }

    /// Drop vectors whose memory is gone or whose model is no longer active.
    pub fn prune_vectors(&self) -> Result<usize> {
        let Some(embedder) = self.embedder() else {
            return Ok(0);
        };
        let removed = store::prune_vectors(&self.inner.session, embedder.model(), embedder.dims())?;
        if removed > 0 {
            self.invalidate_vectors();
        }
        Ok(removed)
    }

    /// The model-facing tools, rendered in UTC.
    pub fn tools(&self) -> Vec<Arc<dyn hearth_tools::Tool>> {
        self.tools_at(None, Zone::utc())
    }

    /// The tools, with the entry this run's writes came from and the configured zone.
    pub fn tools_at(&self, source: Option<String>, zone: Zone) -> Vec<Arc<dyn hearth_tools::Tool>> {
        tools::build(&ToolMemory {
            service: self.clone(),
            source,
            zone,
        })
    }

    fn embedder(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.inner
            .embedder
            .read()
            .ok()
            .and_then(|slot| slot.clone())
    }

    fn embed_for_write(&self, item: &NewMemory) -> Option<store::VectorWrite> {
        let embedder = self.embedder()?;
        let text = if item.subject.is_empty() {
            item.content.clone()
        } else {
            format!("{} {}", item.subject, item.content)
        };
        let values = embedder.embed(&text).ok()?;
        Some(store::VectorWrite {
            model: embedder.model().to_owned(),
            dims: embedder.dims(),
            values,
        })
    }

    fn known_subjects(&self) -> Result<HashSet<String>> {
        Ok(self.subject_list()?.into_iter().collect())
    }

    fn subject_list(&self) -> Result<Vec<String>> {
        if let Ok(cache) = self.inner.subjects.lock()
            && let Some(subjects) = cache.as_ref()
        {
            return Ok(subjects.clone());
        }
        let subjects = store::subjects(&self.inner.session)?;
        if let Ok(mut cache) = self.inner.subjects.lock() {
            *cache = Some(subjects.clone());
        }
        Ok(subjects)
    }

    #[cfg(test)]
    fn cached_vector_count(&self) -> Option<usize> {
        self.inner
            .vectors
            .lock()
            .ok()?
            .as_ref()
            .map(|cache| cache.rows.len())
    }

    fn remember(
        &self,
        outcome: &WriteOutcome,
        vector: Option<&store::VectorWrite>,
        superseded: Option<&str>,
    ) {
        let memory = outcome.memory();
        if !memory.subject.is_empty()
            && let Ok(mut cache) = self.inner.subjects.lock()
            && let Some(subjects) = cache.as_mut()
            && !subjects.iter().any(|subject| subject == &memory.subject)
        {
            subjects.push(memory.subject.clone());
            subjects.sort_by(|left, right| {
                right
                    .chars()
                    .count()
                    .cmp(&left.chars().count())
                    .then_with(|| left.cmp(right))
            });
            subjects.truncate(store::MAX_KNOWN_SUBJECTS as usize);
        }
        let Some(vector) = vector else {
            return;
        };
        let Ok(mut cache) = self.inner.vectors.lock() else {
            return;
        };
        let Some(cache) = cache.as_mut() else {
            return;
        };
        if cache.model != vector.model || cache.dims != vector.dims {
            return;
        }
        let Ok(row) = store::vector_row(&memory.id, vector, true) else {
            return;
        };
        if let Some(old) = superseded
            && old != memory.id
            && let Some(row) = cache.rows.iter_mut().find(|row| row.memory_id == old)
        {
            row.current = false;
        }
        cache.rows.push(row);
    }

    fn invalidate_vectors(&self) {
        if let Ok(mut cache) = self.inner.vectors.lock() {
            *cache = None;
        }
    }
}

/// Add a channel's rows to the pool, keeping the channel's order and each id
/// once.
///
/// A row that two lookups in the same channel both return would otherwise collect
/// a fusion contribution twice and outrank a row found by two different channels,
/// which is not what reciprocal rank means.
fn absorb(items: Vec<Memory>, memories: &mut HashMap<String, Memory>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::with_capacity(items.len());
    for memory in items {
        if !seen.insert(memory.id.clone()) {
            continue;
        }
        ids.push(memory.id.clone());
        memories.insert(memory.id.clone(), memory);
    }
    ids
}

/// The subject a statement suggests, when it opens with a name.
///
/// The agent chooses a subject through the tool, and it should: a person knows
/// that "the board" is the OR board. This is the deterministic fallback the
/// extractor uses, and it is exported so an import can seed a store with the same
/// rule the automatic path applies.
pub fn suggest_subject(content: &str) -> String {
    extract::leading_subject(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearth_core::Harness;
    use tempfile::TempDir;

    fn service() -> (TempDir, MemoryService) {
        let directory = TempDir::new().expect("temp home");
        let harness = Harness::open(directory.path()).expect("harness");
        let service = MemoryService::new(harness.session().clone());
        (directory, service)
    }

    fn seed(service: &MemoryService, content: &str, subject: &str) -> Memory {
        service
            .write(NewMemory::new(content).subject(subject))
            .expect("write")
            .memory()
            .clone()
    }

    #[test]
    fn a_stored_fact_comes_back_in_the_digest() {
        let (_directory, service) = service();
        seed(&service, "Alice manages the OR board.", "alice");
        let recall = service.recall("who runs the OR board").expect("recall");
        assert!(
            recall.digest.contains("Alice manages the OR board."),
            "{}",
            recall.digest
        );
    }

    #[test]
    fn a_pinned_replacement_fits_at_capacity() {
        use crate::errors::Code;

        let (_directory, service) = service();
        let mut ids = Vec::new();
        for index in 0..200 {
            match service.write(
                NewMemory::new(format!("remember rule {index} daily"))
                    .kind(Kind::Rule)
                    .pinned(true),
            ) {
                Ok(outcome) => ids.push(outcome.memory().id.clone()),
                Err(error) if error.code == Code::RulesFull => break,
                Err(error) => panic!("{error}"),
            }
        }
        assert!(ids.len() > 1, "the store must fill before the replacement");

        let replaced = ids[0].clone();
        let outcome = service.write(
            NewMemory::new("remember rule x daily")
                .kind(Kind::Rule)
                .pinned(true)
                .supersedes(&replaced),
        );
        assert!(
            outcome.is_ok(),
            "a replacement frees its own space: {:?}",
            outcome.err()
        );
        let old = service.get(&replaced).expect("get").expect("row");
        assert!(!old.is_current(), "the replaced rule must retire");
    }

    struct ConstantEmbedding;

    impl EmbeddingProvider for ConstantEmbedding {
        fn model(&self) -> &str {
            "fake-model"
        }

        fn dims(&self) -> usize {
            4
        }

        fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![1.0, 0.0, 0.0, 0.0])
        }
    }

    #[test]
    fn the_backfill_indexes_every_pending_memory() {
        let (_directory, service) = service();
        for index in 0..300 {
            seed(&service, &format!("fact number {index}"), "topic");
        }
        let service = service.with_embedder(Arc::new(ConstantEmbedding));

        let indexed = service.index_pending_vectors().expect("backfill");

        assert_eq!(indexed, 300, "every pending row gets a vector");
        assert!(
            store::pending_vectors(&service.inner.session, "fake-model", 4, 1)
                .expect("pending")
                .is_empty(),
            "no row is left for a later pass"
        );
    }

    #[test]
    fn a_write_extends_the_vector_cache() {
        let (_directory, service) = service();
        let service = service.with_embedder(Arc::new(ConstantEmbedding));
        seed(&service, "alpha fact", "alpha");
        seed(&service, "beta fact", "beta");
        service.recall("alpha beta").expect("recall");
        assert_eq!(service.cached_vector_count(), Some(2));

        seed(&service, "gamma fact", "gamma");

        assert_eq!(
            service.cached_vector_count(),
            Some(3),
            "a write must extend the loaded vector cache, not discard it"
        );
    }

    #[test]
    fn the_memory_default_rules_budget_is_800() {
        assert_eq!(MemoryConfig::default().rules_max_tokens, 800);
    }

    #[test]
    fn the_memory_recall_default_is_10_items() {
        let config = MemoryConfig::default();
        assert_eq!(config.recall_max_items, 10);
    }

    #[test]
    fn ten_memories_at_the_content_limit_fit_the_default_digest() {
        let (_directory, service) = service();
        let mut subjects = Vec::new();
        for index in 0..10 {
            let subject = format!("topic{index}");
            let content = format!("{} note {index}", "x".repeat(CONTENT_LIMIT - 7));
            assert_eq!(content.chars().count(), CONTENT_LIMIT);
            seed(&service, &content, &subject);
            subjects.push(subject);
        }

        let recall = service.recall(&subjects.join(" ")).expect("recall");
        assert_eq!(recall.items.len(), 10, "{}", recall.digest);
        let tokens = hearth_model::estimate_tokens(&recall.digest);
        assert!(
            tokens <= 1_200,
            "ten full rows must stay under the count bound's ceiling: {tokens}\n{}",
            recall.digest
        );
        println!("ten full rows: {tokens} tokens");
    }

    #[test]
    fn the_worst_case_digest_stays_under_its_ceiling() {
        let (_directory, service) = service();
        let main = "m".repeat(120);
        let long_a = "s".repeat(120);
        let long_b = "t".repeat(120);
        for (index, subject) in [main.as_str(), long_a.as_str(), long_b.as_str()]
            .iter()
            .enumerate()
        {
            let old = seed(
                &service,
                &format!("{} old note {index}", "o".repeat(CONTENT_LIMIT - 20)),
                subject,
            );
            service
                .write(
                    NewMemory::new(format!(
                        "{} new note {index}",
                        "n".repeat(CONTENT_LIMIT - 20)
                    ))
                    .subject(*subject)
                    .supersedes(old.id.clone()),
                )
                .expect("replace");
        }
        for index in 0..10 {
            seed(
                &service,
                &format!("note {index} {}", "x".repeat(CONTENT_LIMIT - 12)),
                &main,
            );
        }

        let message = format!("{main} {long_a} {long_b}");
        let recall = service.recall(&message).expect("recall");
        let tokens = hearth_model::estimate_tokens(&recall.digest);
        assert_eq!(recall.items.len(), 10, "{}", recall.digest);
        assert!(recall.digest.contains("more:"), "{}", recall.digest);
        assert!(recall.digest.contains("earlier:"), "{}", recall.digest);
        assert!(
            tokens <= 1_200,
            "the worst-case digest must stay under the count bound's ceiling: {tokens}\n{}",
            recall.digest
        );
        println!("worst-case digest: {tokens} tokens");
    }

    #[test]
    fn every_live_rule_fits_in_the_rules_block() {
        use crate::errors::Code;

        let (_directory, service) = service();
        let mut written = 0;
        for index in 0..200 {
            match service.write(
                NewMemory::new(format!("remember rule {index} daily"))
                    .kind(Kind::Rule)
                    .pinned(true),
            ) {
                Ok(_) => written += 1,
                Err(error) if error.code == Code::RulesFull => break,
                Err(error) => panic!("{error}"),
            }
        }
        assert!(written > 1, "the store must admit more than one rule");

        let block = service.rules().expect("rules").text;
        assert!(
            !block.contains("more rules were not shown"),
            "every live rule must fit the block: {block}"
        );
        let shown = block.lines().count();
        assert_eq!(
            shown, written,
            "the store admitted {written} rules but the block shows {shown}\n{block}"
        );
    }

    #[test]
    fn a_question_naming_a_subject_reports_what_was_left_out() {
        let (_directory, service) = service();
        let service = service.with_config(MemoryConfig {
            recall_max_items: 4,
            ..MemoryConfig::default()
        });
        for (index, topic) in [
            "hiring", "budget", "roadmap", "launch", "design", "support", "security", "pricing",
        ]
        .iter()
        .enumerate()
        {
            seed(
                &service,
                &format!("Alice owns the {topic} track for cycle {index}"),
                "alice",
            );
        }
        let recall = service.recall("what does Alice own").expect("recall");
        assert!(
            recall
                .coverage
                .current
                .get("alice")
                .is_some_and(|n| *n >= 8),
            "{:?}",
            recall.coverage
        );
        assert!(
            recall.digest.contains("more: alice"),
            "the digest must admit the subset: {}",
            recall.digest
        );
    }

    #[test]
    fn a_greeting_recalls_nothing_and_renders_nothing() {
        let (_directory, service) = service();
        seed(&service, "a fact about a topic", "topic");
        let recall = service.recall("hello").expect("recall");
        assert!(recall.items.is_empty());
        assert!(recall.digest.is_empty());
    }

    #[test]
    fn a_replacement_stops_the_old_answer_from_leading() {
        let (_directory, service) = service();
        let old = seed(
            &service,
            "the primary database is Postgres",
            "primary database",
        );
        service
            .write(
                NewMemory::new("the primary database is SQLite")
                    .subject("primary database")
                    .supersedes(old.id.clone()),
            )
            .expect("replace");
        let recall = service
            .recall("what is the primary database")
            .expect("recall");
        assert!(recall.digest.contains("SQLite"), "{}", recall.digest);
        assert!(
            !recall.digest.contains("Postgres"),
            "a replaced statement must not lead the digest: {}",
            recall.digest
        );
        assert!(
            recall.digest.contains("earlier: primary database"),
            "{}",
            recall.digest
        );
    }

    #[test]
    fn a_rule_is_injected_and_versioned() {
        let (_directory, service) = service();
        let before = service.rules().expect("rules");
        service
            .write(
                NewMemory::new("Never use em dashes.")
                    .kind(Kind::Rule)
                    .pinned(true),
            )
            .expect("rule");
        let after = service.rules().expect("rules");
        assert!(
            after.text.contains("Never use em dashes."),
            "{}",
            after.text
        );
        assert_ne!(before.version, after.version);
        assert_eq!(after.count, 1);
    }

    #[test]
    fn the_rules_budget_is_enforced_at_the_write() {
        let (_directory, service) = service();
        let service = service.with_config(MemoryConfig {
            rules_max_tokens: 50,
            ..MemoryConfig::default()
        });
        service
            .write(
                NewMemory::new("Keep answers short.")
                    .kind(Kind::Rule)
                    .pinned(true),
            )
            .expect("first");
        let error = service
            .write(
                NewMemory::new("b".repeat(CONTENT_LIMIT))
                    .kind(Kind::Rule)
                    .pinned(true),
            )
            .expect_err("full");
        assert_eq!(error.code, Code::RulesFull);
        assert!(
            error.message.contains("replace or forget one of m_"),
            "the refusal must name what to replace: {}",
            error.message
        );
    }

    #[test]
    fn an_unpinned_copy_does_not_retire_a_pinned_rule() {
        let (_directory, service) = service();
        let rule = service
            .write(
                NewMemory::new("Never deploy on a Friday.")
                    .kind(Kind::Rule)
                    .pinned(true),
            )
            .expect("rule")
            .memory()
            .clone();
        assert_eq!(service.rules().expect("rules").count, 1);

        let copy = service
            .write(NewMemory::new("Never deploy on a Friday.").kind(Kind::Commitment))
            .expect("copy");

        assert!(
            !copy.created(),
            "the pinned rule already carries the statement"
        );
        assert_eq!(copy.memory().id, rule.id);
        assert!(copy.memory().pinned, "the stored statement is still a rule");
        let rules = service.rules().expect("rules");
        assert_eq!(rules.count, 1, "the pinned rule must stay injected");
        assert!(rules.text.contains("Friday"), "{}", rules.text);
        let stored = service.get(&rule.id).expect("get").expect("the rule row");
        assert!(stored.is_current(), "the copy must not retire the rule");
        assert!(stored.pinned, "the live rule must stay pinned");
        assert_eq!(service.count().expect("count"), 1, "the copy is not a row");
    }

    #[test]
    fn an_explicit_replacement_still_retires_a_pinned_rule() {
        let (_directory, service) = service();
        let rule = service
            .write(
                NewMemory::new("Never deploy on a Friday.")
                    .kind(Kind::Rule)
                    .pinned(true),
            )
            .expect("rule")
            .memory()
            .clone();

        let replacement = service
            .write(
                NewMemory::new("Never deploy on a Friday.")
                    .kind(Kind::Commitment)
                    .supersedes(rule.id.clone()),
            )
            .expect("replacement");

        assert!(replacement.created(), "the named replacement is a new row");
        assert_eq!(service.rules().expect("rules").count, 0);
        let retired = service.get(&rule.id).expect("get").expect("the old row");
        assert!(
            !retired.is_current(),
            "an explicit replacement retires the rule"
        );
        assert!(!retired.pinned, "a retired row cannot stay pinned");
    }

    #[test]
    fn a_retired_rule_stops_holding_the_rules_budget() {
        let (_directory, service) = service();
        let retired_text = "Always write the changelog entry before the release tag and cite \
                             every merged change with the risk in plain words so the record \
                             reads clearly now.";
        let live_text = "Never publish a release tag without the changelog entry that cites \
                         each merged change and the risk in plain words so the record reads \
                         clearly now.";
        assert_eq!(
            retired_text.chars().count(),
            live_text.chars().count(),
            "the two rules must cost the same so the budget is exact"
        );
        let rule = service
            .write(NewMemory::new(retired_text).kind(Kind::Rule).pinned(true))
            .expect("rule")
            .memory()
            .clone();
        service
            .write(
                NewMemory::new(retired_text)
                    .kind(Kind::Commitment)
                    .supersedes(rule.id.clone()),
            )
            .expect("retire");
        let service = service.with_config(MemoryConfig {
            rules_max_tokens: crate::digest::rules_cost(&[&rule]),
            ..MemoryConfig::default()
        });
        service
            .write(NewMemory::new(live_text).kind(Kind::Rule).pinned(true))
            .expect("the retired rule must not hold the budget");
        let error = service
            .write(
                NewMemory::new(
                    "Keep every release note short enough to read in one sitting and cite each change.",
                )
                .kind(Kind::Rule)
                .pinned(true),
            )
            .expect_err("the live rule holds the budget");
        assert_eq!(error.code, Code::RulesFull);
    }

    #[test]
    fn a_rule_longer_than_one_line_is_refused() {
        let (_directory, service) = service();
        let error = service
            .write(
                NewMemory::new("r".repeat(401))
                    .kind(Kind::Rule)
                    .pinned(true),
            )
            .expect_err("too long");
        assert_eq!(error.code, Code::InvalidInput);
    }

    #[test]
    fn forgetting_removes_the_row() {
        let (_directory, service) = service();
        let memory = seed(&service, "ephemeral", "ephemeral");
        service.forget(&memory.id).expect("forget");
        assert!(service.get(&memory.id).expect("get").is_none());
    }

    #[test]
    fn the_tool_set_is_four_tools() {
        let (_directory, service) = service();
        let names: Vec<String> = service.tools().iter().map(|t| t.spec().name).collect();
        assert_eq!(names, vec![FORGET, RECALL, SAVE, TRANSCRIPT]);
    }
    #[test]
    fn an_existing_row_still_renders() {
        let (_directory, service) = service();
        let row = seed(&service, "bob likes apples", "bob");
        let id = row.id.clone();
        service
            .session()
            .storage()
            .write(move |writer| {
                writer.execute(
                    "UPDATE memories SET content = 'he likes apples', terms = 'he likes apples' \
                     WHERE id = ?",
                    [id],
                )
            })
            .expect("a row written by an earlier release");
        let recall = service.recall("apples").expect("recall");
        assert!(
            recall.digest.contains("he likes apples"),
            "recall must not hide an existing row: {}",
            recall.digest
        );
    }
}
