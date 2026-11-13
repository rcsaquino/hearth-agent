//! Hearth memory.
//!
//! Two stores, one design:
//!
//! * **standing memory** is `standing.md`, a plain file of prioritised bullets
//!   injected before every prompt. It is for requirements that must be present
//!   without a retrieval cue, capped at [`STANDING_LIMIT`] characters, and the
//!   only way to make room is to displace a strictly lower-priority bullet,
//!   which is archived to the database rather than dropped.
//! * **long-term memory** is a SQLite database with an FTS5 index over content,
//!   tags, and provenance, using a Porter-stemming tokenizer. A query the index
//!   cannot answer falls back to a literal substring scan, so a phrase that is
//!   plainly present is never reported as absent.
//!
//! The agent also performs automatic recall for each user message: lexical FTS5
//! and, when the local model is available, a local vector index are fused with
//! reciprocal rank. Automatic recall is capped to short snippets. The model can
//! still reach the full stores through five tools: add, edit, delete, search, and
//! a history search over the conversation transcript.

mod database;
mod embedding;
mod errors;
mod files;
mod history;
mod results;
mod schema;
mod standing;
mod text;
mod tools;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use hearth_core::Session;

pub use database::{
    Memory, MemoryInput, MemoryRead, MemorySearchOptions, MemoryStore, SearchHit, SearchMode,
    SearchResponse,
};
pub use embedding::{
    BekkoEmbedding, EmbeddingProvider, MODEL_DIMS, MODEL_NAME, download_model, model_files_present,
};
pub use errors::{ErrorCode, ExpectedError, Result};
pub use history::{
    HistoryHit, HistoryMode, HistoryQuery, HistoryRead, HistoryResponse, HistorySearch,
};
pub use results::{
    render_history_read, render_history_search, render_memory_delete, render_memory_read,
    render_memory_write, render_search, render_standing_add, render_standing_delete,
    render_standing_edit,
};
pub use standing::{
    ArchiveFn, StandingAddResult, StandingDeleteResult, StandingEditResult, StandingItem,
    StandingSnapshot, StandingStore,
};
pub use text::{MEMORY_LIMIT, NEW_MEMORY_LIMIT, NEW_TAG_LIMIT, RESULT_BYTES, STANDING_LIMIT};
pub use tools::{AddTool, DeleteTool, EditTool, HistoryTool, MemoryTools, SearchTool};

/// The longest remembered text injected by automatic recall.
const RECALL_ITEM_CHARS: usize = 240;

/// The memory subsystem handed to a run.
///
/// Holds the three stores and the provenance stamped onto saves. All methods are
/// cheap: the database is opened once and reused, and the standing file is read
/// on demand so a hand edit takes effect on the next turn.
pub struct MemoryService {
    store: Arc<MemoryStore>,
    standing: Arc<StandingStore>,
    history: Arc<HistorySearch>,
    source: Option<String>,
    base: PathBuf,
    recall_max_items: u64,
    recall_max_tokens: u64,
    embeddings_configured: AtomicBool,
    embeddings_active: AtomicBool,
    embedding_model: Mutex<Option<String>>,
}

impl MemoryService {
    /// Open memory beneath a base directory.
    ///
    /// The layout is two files: `memory/long-term.sqlite` and
    /// `memory/standing.md`. Keeping them together means the whole memory can be
    /// copied, backed up, or inspected as one directory.
    pub fn open(base: impl AsRef<Path>) -> Result<Self> {
        let root = base.as_ref().to_path_buf();
        let base = root.join("memory");
        std::fs::create_dir_all(&base)?;
        let database_path = base.join("long-term.sqlite");
        let standing_path = base.join("standing.md");
        // The transcript lives in the session database beside the memory
        // directory, not in the memory database itself. Pointing the history
        // reader at the wrong file would make every history search fail on a
        // missing `entries` table.
        let transcript_path = root.join("hearth.db");
        let store = Arc::new(MemoryStore::open(&database_path)?);
        let store_for_archive = Arc::clone(&store);
        let standing_source = standing_path.display().to_string();
        let standing = Arc::new(StandingStore::new(
            &standing_path,
            Arc::new(move |items: &[StandingItem]| {
                store_for_archive
                    .archive(items, &standing_source)
                    .map(|_| ())
            }),
        ));
        Ok(Self {
            store,
            standing,
            history: Arc::new(HistorySearch::new(transcript_path)),
            source: None,
            base,
            recall_max_items: 8,
            recall_max_tokens: 900,
            embeddings_configured: AtomicBool::new(false),
            embeddings_active: AtomicBool::new(false),
            embedding_model: Mutex::new(None),
        })
    }

    /// Open memory for a session, using the directory holding its database.
    pub fn new(session: Session) -> Result<Self> {
        let base = session
            .storage()
            .path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::open(base)
    }

    /// Record the provenance stamped onto saves that do not name one.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// The directory holding both stores.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// The long-term store.
    pub fn store(&self) -> &Arc<MemoryStore> {
        &self.store
    }

    /// The standing store.
    pub fn standing_store(&self) -> &Arc<StandingStore> {
        &self.standing
    }

    /// The transcript reader.
    pub fn history(&self) -> &Arc<HistorySearch> {
        &self.history
    }

    /// Read the standing memory text and a version for prompt caching.
    ///
    /// The version changes whenever the file content changes, which is what lets
    /// the prompt builder notice that a cached prefix is stale.
    pub fn standing(&self) -> Result<(String, u64)> {
        let text = self.standing.text()?;
        let version = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut hasher);
            hasher.finish()
        };
        Ok((text, version))
    }

    /// The rendered standing memory for the prompt.
    ///
    /// An empty store renders empty so the prompt segment collapses rather than
    /// announcing an empty section.
    pub fn render_standing(&self) -> String {
        self.standing().map(|(text, _)| text).unwrap_or_default()
    }

    /// Apply the configured recall bounds.
    pub fn with_recall_budget(mut self, max_items: u64, max_tokens: u64) -> Self {
        self.recall_max_items = max_items.clamp(1, 50);
        self.recall_max_tokens = max_tokens.max(1);
        self
    }

    /// Record whether semantic recall is configured. The backend reports when
    /// its index is actually ready.
    pub fn with_embeddings(self, configured: bool) -> Self {
        self.embeddings_configured
            .store(configured, Ordering::Relaxed);
        self
    }

    /// Attach a ready local embedding provider.
    pub fn with_embedder(self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        self.activate_embedder(embedder);
        self
    }

    /// Attach an embedder after the daemon has already started.
    pub fn activate_embedder(&self, embedder: Arc<dyn EmbeddingProvider>) {
        self.embeddings_configured.store(true, Ordering::Relaxed);
        if let Ok(mut model) = self.embedding_model.lock() {
            *model = Some(embedder.model().to_owned());
        }
        self.store.set_embedder(embedder);
        self.embeddings_active.store(true, Ordering::Relaxed);
    }

    /// Index memories that are missing a vector for the active model.
    pub fn index_pending_vectors(&self) -> Result<usize> {
        self.store.index_pending_vectors()
    }

    /// The recall path currently in use.
    pub fn recall_mode(&self) -> &'static str {
        if self.embeddings_active() {
            "hybrid"
        } else {
            "lexical"
        }
    }

    pub fn recall_max_items(&self) -> u64 {
        self.recall_max_items
    }

    pub fn recall_max_tokens(&self) -> u64 {
        self.recall_max_tokens
    }

    pub fn embeddings_configured(&self) -> bool {
        self.embeddings_configured.load(Ordering::Relaxed)
    }

    pub fn embeddings_active(&self) -> bool {
        self.embeddings_active.load(Ordering::Relaxed)
    }

    pub fn embedding_model(&self) -> Option<String> {
        self.embedding_model
            .lock()
            .ok()
            .and_then(|model| model.clone())
    }

    /// Retrieve memories relevant to a query.
    ///
    /// This is a keyword search with stemming, plus the literal fallback. It is
    /// deliberately not a semantic search: relevance comes from the words the
    /// caller chose, and an empty query retrieves nothing rather than everything.
    pub fn recall(&self, query: &str) -> Result<Vec<Memory>> {
        let trimmed = query.trim();
        if trimmed.is_empty() || is_trivial(trimmed) {
            return Ok(Vec::new());
        }
        let response = self.store.search(
            trimmed,
            SearchMode::Any,
            self.recall_max_items as i64,
            0,
            &MemorySearchOptions::default(),
        )?;
        Ok(response.results.into_iter().map(|hit| hit.memory).collect())
    }

    /// Render recalled memories as prompt lines within the token budget.
    ///
    /// Tags stay in the search tool output; automatic recall spends its budget
    /// on remembered text only. A single oversized hit is clipped rather than
    /// discarded.
    pub fn render_recall(&self, items: &[Memory]) -> String {
        let mut rendered = String::new();
        let mut used = 0u64;
        for item in items {
            let content = clip_chars(&item.content.replace('\n', " "), RECALL_ITEM_CHARS);
            let bullet = format!("- {content}");
            let cost = hearth_model::estimate_tokens(&bullet).max(0) as u64;
            if used > 0 && used + cost > self.recall_max_tokens {
                break;
            }
            if used == 0 && cost > self.recall_max_tokens {
                let max_chars = (self.recall_max_tokens.saturating_mul(3)) as usize;
                rendered.push_str(&clip_chars(&bullet, max_chars));
                break;
            }
            if !rendered.is_empty() {
                rendered.push('\n');
            }
            rendered.push_str(&bullet);
            used += cost;
            if used >= self.recall_max_tokens {
                break;
            }
        }
        rendered
    }

    /// The tools for this store, with the run's provenance attached.
    pub fn tools(&self) -> Vec<Arc<dyn hearth_tools::Tool>> {
        let tools = MemoryTools::new(
            Arc::clone(&self.standing),
            Arc::clone(&self.store),
            self.history.path().to_path_buf(),
        );
        let tools = match &self.source {
            Some(source) => tools.with_source(source.clone()),
            None => tools,
        };
        vec![
            Arc::new(AddTool::new(tools.clone())),
            Arc::new(EditTool::new(tools.clone())),
            Arc::new(DeleteTool::new(tools.clone())),
            Arc::new(SearchTool::new(tools.clone())),
            Arc::new(HistoryTool::new(tools)),
        ]
    }
}

fn clip_chars(text: &str, max: usize) -> String {
    let mut clipped: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        clipped.push('…');
    }
    clipped
}

/// Inputs that never deserve a memory recall.
///
/// A greeting carries no topic, so searching for it spends tokens to return
/// nothing useful. This is a fixed list rather than a heuristic so the behavior
/// is predictable.
pub fn is_trivial(text: &str) -> bool {
    const TRIVIAL: &[&str] = &[
        "hi",
        "hello",
        "hey",
        "thanks",
        "thank you",
        "ok",
        "okay",
        "sure",
        "yes",
        "no",
        "good morning",
        "good afternoon",
        "good evening",
        "lol",
    ];
    let normalized = text
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != ' ');
    if normalized.is_empty() {
        return true;
    }
    let lowered = normalized.to_lowercase();
    TRIVIAL.contains(&lowered.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeEmbedding;

    impl EmbeddingProvider for FakeEmbedding {
        fn model(&self) -> &str {
            "fake"
        }

        fn dims(&self) -> usize {
            2
        }

        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            let lower = text.to_lowercase();
            let mut vector = vec![0.0f32; 2];
            if lower.contains("paint") || lower.contains("art") {
                vector[0] = 1.0;
            }
            if lower.contains("soup") || lower.contains("food") {
                vector[1] = 1.0;
            }
            if vector.iter().all(|value| *value == 0.0) {
                vector[0] = 0.01;
            }
            let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
            for value in &mut vector {
                *value /= norm;
            }
            Ok(vector)
        }
    }

    fn service() -> (tempfile::TempDir, MemoryService) {
        let directory = tempfile::tempdir().unwrap();
        let service = MemoryService::open(directory.path()).unwrap();
        (directory, service)
    }

    #[test]
    #[ignore = "requires HEARTH_EMBEDDING_MODEL_DIR"]
    fn the_local_embedding_model_smoke_runs() {
        let directory =
            std::env::var("HEARTH_EMBEDDING_MODEL_DIR").expect("set HEARTH_EMBEDDING_MODEL_DIR");
        let model = BekkoEmbedding::open(directory)
            .expect("open model")
            .expect("model files");
        let vector = model.embed("Alice manages Bob").expect("embed");
        assert_eq!(vector.len(), MODEL_DIMS);
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001, "norm was {norm}");
    }

    #[test]
    #[ignore = "requires HEARTH_EMBEDDING_MODEL_DIR"]
    fn the_local_model_finds_a_paraphrase() {
        let directory =
            std::env::var("HEARTH_EMBEDDING_MODEL_DIR").expect("set HEARTH_EMBEDDING_MODEL_DIR");
        let model = BekkoEmbedding::open(directory)
            .expect("open model")
            .expect("model files");
        let (_directory, service) = service();
        let service = service.with_embedder(Arc::new(model));
        service
            .store()
            .add(&MemoryInput {
                content: "Alice manages Bob".to_owned(),
                tags: Some(vec!["work".to_owned()]),
                ..Default::default()
            })
            .unwrap();
        service
            .store()
            .add(&MemoryInput {
                content: "Alice prefers sour soup".to_owned(),
                tags: Some(vec!["food".to_owned()]),
                ..Default::default()
            })
            .unwrap();
        let started = std::time::Instant::now();
        let items = service.recall("Who is Bob's boss?").unwrap();
        println!(
            "paraphrase recall took {:?}: {:?}",
            started.elapsed(),
            items.iter().map(|item| &item.content).collect::<Vec<_>>()
        );
        assert!(
            items
                .iter()
                .any(|item| item.content.contains("manages Bob")),
            "paraphrase recall missed the manager memory"
        );
    }

    #[test]
    fn the_layout_is_two_files_in_one_directory() {
        let (directory, service) = service();
        assert!(directory.path().join("memory").is_dir());
        assert!(service.store().path().ends_with("long-term.sqlite"));
        assert!(service.standing_store().path().ends_with("standing.md"));
    }

    #[test]
    fn a_saved_fact_is_reachable_by_recall() {
        let (_directory, service) = service();
        service
            .store()
            .add(&MemoryInput {
                content: "USER_NAME is allergic to nuts".to_owned(),
                tags: Some(vec!["health".to_owned()]),
                source: Some("test".to_owned()),
            })
            .unwrap();
        let items = service.recall("nuts").unwrap();
        assert_eq!(items.len(), 1);
        let rendered = service.render_recall(&items);
        assert!(rendered.contains("nuts"));
        assert!(
            !rendered.contains("[health]"),
            "automatic recall spends its budget on text, not tags: {rendered}"
        );
    }

    #[test]
    fn automatic_recall_stays_within_its_token_budget() {
        let (_directory, service) = service();
        for index in 0..4 {
            service
                .store()
                .add(&MemoryInput {
                    content: format!("needle fact {index} {}", "x".repeat(1_800)),
                    ..Default::default()
                })
                .unwrap();
        }
        let items = service.recall("needle").unwrap();
        let rendered = service.render_recall(&items);
        let tokens = hearth_model::estimate_tokens(&rendered);
        assert!(
            tokens <= 900,
            "recall rendered {tokens} tokens, over the default budget"
        );
        assert!(
            rendered.lines().count() <= 8,
            "recall returned too many items: {rendered}"
        );
    }

    #[test]
    fn an_attached_embedder_turns_on_hybrid_recall() {
        let (_directory, service) = service();
        assert_eq!(service.recall_mode(), "lexical");
        assert!(!service.embeddings_active());
        let service = service
            .with_embeddings(true)
            .with_embedder(Arc::new(FakeEmbedding));
        assert_eq!(service.recall_mode(), "hybrid");
        assert!(service.embeddings_configured());
        assert!(service.embeddings_active());
        assert_eq!(service.embedding_model().as_deref(), Some("fake"));
        service
            .store()
            .add(&MemoryInput {
                content: "she was painting a sunrise".to_owned(),
                ..Default::default()
            })
            .unwrap();
        let items = service.recall("artwork").unwrap();
        assert!(
            items
                .iter()
                .any(|memory| memory.content.contains("painting")),
            "hybrid recall missed the vector match"
        );
    }

    #[test]
    fn an_embedder_can_be_activated_after_startup() {
        let (_directory, service) = service();
        let service = service.with_embeddings(true);
        assert!(!service.embeddings_active());
        assert_eq!(service.recall_mode(), "lexical");
        service.activate_embedder(Arc::new(FakeEmbedding));
        assert!(service.embeddings_active());
        assert_eq!(service.recall_mode(), "hybrid");
        assert_eq!(service.embedding_model().as_deref(), Some("fake"));
    }

    #[test]
    fn a_trivial_input_recalls_nothing() {
        let (_directory, service) = service();
        service
            .store()
            .add(&MemoryInput {
                content: "a fact".to_owned(),
                ..Default::default()
            })
            .unwrap();
        assert!(service.recall("hello").unwrap().is_empty());
        assert!(service.recall("   ").unwrap().is_empty());
    }

    #[test]
    fn a_standing_bullet_is_injected_and_versioned() {
        let (_directory, service) = service();
        let empty_version = service.standing().unwrap().1;
        service
            .standing_store()
            .add("Never use em dashes.", 90)
            .unwrap();
        let (text, version) = service.standing().unwrap();
        assert!(text.contains("Never use em dashes."));
        assert_ne!(version, empty_version);
    }

    #[test]
    fn a_hand_edit_to_the_standing_file_is_picked_up() {
        let (directory, service) = service();
        std::fs::write(
            directory.path().join("memory/standing.md"),
            "- I prefer plain English.\n",
        )
        .unwrap();
        assert!(
            service
                .render_standing()
                .contains("I prefer plain English.")
        );
    }

    #[test]
    fn the_tool_set_is_the_five_memory_tools() {
        let (_directory, service) = service();
        let names: Vec<String> = service
            .tools()
            .iter()
            .map(|tool| tool.spec().name)
            .collect();
        assert_eq!(
            names,
            vec![
                "memory_add",
                "memory_edit",
                "memory_delete",
                "memory_search",
                "memory_sessions"
            ]
        );
    }

    #[test]
    fn trivial_inputs_are_recognized() {
        assert!(is_trivial("hello"));
        assert!(is_trivial("  Hello!  "));
        assert!(is_trivial(""));
        assert!(!is_trivial("what did we decide about the OR board"));
    }
}
