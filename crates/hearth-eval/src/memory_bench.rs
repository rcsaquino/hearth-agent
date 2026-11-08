//! The offline memory recall benchmark.
//!
//! The fixture is a synthetic library and a fixed set of questions. The
//! benchmark reports the 1.0 targets next to the measured baseline; it does
//! not hide a miss. Only leakage and duplicate-write failures are hard
//! invariants.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use hearth_core::{Clock, Harness, UuidV7};
use hearth_memory::{
    MemoryError, MemoryKind, MemoryService, MemoryStatus, NewMemoryInput, SourceKind,
};
use hearth_model::estimate_tokens;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

const FIXTURE: &str = include_str!("../fixtures/memory_bench.json");
const FIXED_NOW: i64 = 1_735_689_600_000_000; // 2025-01-01T00:00:00Z
const TARGET_RECALL: f64 = 0.90;
const TARGET_PRECISION: f64 = 0.80;
const TARGET_TOKENS: usize = 900;
const TARGET_RETRIEVAL_MS: f64 = 80.0;

/// The fixed clock used for the synthetic history.
struct FixedClock;

impl Clock for FixedClock {
    fn now_micros(&self) -> i64 {
        FIXED_NOW
    }
}

#[derive(Debug, Deserialize)]
struct Fixture {
    version: u32,
    memories: Vec<FixtureMemory>,
    questions: Vec<FixtureQuestion>,
    secrets: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct FixtureMemory {
    key: String,
    content: String,
    kind: String,
    tags: Vec<String>,
    importance: i64,
    valid_from: Option<i64>,
    valid_to: Option<i64>,
    supersedes_key: Option<String>,
    duplicate_of: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FixtureQuestion {
    key: String,
    question: String,
    expected_keys: Vec<String>,
    relevant_keys: Vec<String>,
    category: String,
    as_of_micros: Option<i64>,
    spot_check: bool,
}

/// The measured benchmark.
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub fixture_version: u32,
    pub questions: usize,
    pub spot_check: usize,
    pub fixture_memories: usize,
    pub stored_rows: usize,
    pub recall_at_10: f64,
    pub precision_at_5: f64,
    pub recall_tokens_p95: usize,
    pub retrieval_ms_p95: f64,
    pub categories: BTreeMap<String, CategoryReport>,
    pub paraphrase: Option<CategoryReport>,
    pub temporal: TemporalReport,
    pub supersession: SupersessionReport,
    pub duplicate_failures: usize,
    pub secret_leaks: usize,
    pub targets: BenchTargets,
    pub scale: Option<ScaleReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryReport {
    pub count: usize,
    pub recall_at_10: f64,
    pub precision_at_5: f64,
    pub recall_tokens_p95: usize,
    pub retrieval_ms_p95: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TemporalReport {
    pub count: usize,
    pub correct: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SupersessionReport {
    pub count: usize,
    pub correct: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchTargets {
    pub recall_at_10: f64,
    pub precision_at_5: f64,
    pub recall_tokens_p95: usize,
    pub retrieval_ms_p95: f64,
    pub secret_leaks: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScaleReport {
    pub items: usize,
    pub fixture_rows: usize,
    pub noise_rows: usize,
    pub queries: usize,
    pub matched: usize,
    pub search_ms_p95: f64,
    pub standing_ms_p95: f64,
    pub retrieval_ms_p95: f64,
    pub real_recall_ms_p95: f64,
    pub target_ms: f64,
}

#[derive(Default)]
struct Acc {
    count: usize,
    hits: usize,
    precision_sum: f64,
    tokens: Vec<usize>,
    latencies: Vec<f64>,
}

impl Acc {
    fn finish(&self) -> CategoryReport {
        CategoryReport {
            count: self.count,
            recall_at_10: ratio(self.hits, self.count),
            precision_at_5: if self.count == 0 {
                0.0
            } else {
                self.precision_sum / self.count as f64
            },
            recall_tokens_p95: p95_usize(&self.tokens),
            retrieval_ms_p95: p95_f64(&self.latencies),
        }
    }
}

struct LoadedFixture {
    key_to_id: BTreeMap<String, String>,
    id_to_keys: BTreeMap<String, Vec<String>>,
    duplicate_failures: usize,
    stored_rows: usize,
}

/// Load the fixture memories and map their keys to stored ids.
fn load_fixture(service: &MemoryService, fixture: &Fixture) -> Result<LoadedFixture> {
    let mut key_to_id: BTreeMap<String, String> = BTreeMap::new();
    let mut id_to_keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut duplicate_failures = 0usize;
    for memory in &fixture.memories {
        let kind = MemoryKind::parse(&memory.kind)
            .with_context(|| format!("unknown memory kind for {}", memory.key))?;
        let supersedes = match memory.supersedes_key.as_deref() {
            Some(old_key) => Some(
                key_to_id
                    .get(old_key)
                    .cloned()
                    .with_context(|| format!("missing superseded key {old_key}"))?,
            ),
            None => None,
        };
        let item = service
            .save(NewMemoryInput {
                content: memory.content.clone(),
                kind,
                tags: memory.tags.clone(),
                importance: memory.importance,
                source_kind: SourceKind::Session,
                source_ref: Some(memory.key.clone()),
                supersedes,
                valid_from: memory.valid_from,
                valid_to: memory.valid_to,
            })
            .with_context(|| format!("save {}", memory.key))?;
        if let Some(original_key) = memory.duplicate_of.as_deref() {
            let original = key_to_id
                .get(original_key)
                .with_context(|| format!("missing duplicate original {original_key}"))?;
            if &item.id != original {
                duplicate_failures += 1;
            }
        }
        key_to_id.insert(memory.key.clone(), item.id.clone());
        id_to_keys
            .entry(item.id.clone())
            .or_default()
            .push(memory.key.clone());
    }
    let stored_rows = service.stats().context("read memory stats")?.total as usize;
    Ok(LoadedFixture {
        key_to_id,
        id_to_keys,
        duplicate_failures,
        stored_rows,
    })
}

/// Run the benchmark over the versioned fixture.
pub fn run() -> Result<BenchReport> {
    let fixture: Fixture = serde_json::from_str(FIXTURE).context("parse the memory fixture")?;
    let dir = TempDir::new().context("create the benchmark home")?;
    let harness = Harness::open_with(dir.path(), Arc::new(FixedClock), Arc::new(UuidV7))
        .context("open the benchmark record")?;
    let service = MemoryService::new(harness.session().clone());

    let loaded = load_fixture(&service, &fixture)?;
    let key_to_id = loaded.key_to_id;
    let id_to_keys = loaded.id_to_keys;
    let duplicate_failures = loaded.duplicate_failures;
    let stored_rows = loaded.stored_rows;

    let mut secret_leaks = 0usize;
    for secret in &fixture.secrets {
        match service.save(NewMemoryInput {
            content: secret.clone(),
            kind: MemoryKind::Fact,
            tags: Vec::new(),
            importance: 10,
            source_kind: SourceKind::External,
            source_ref: None,
            supersedes: None,
            valid_from: None,
            valid_to: None,
        }) {
            Ok(_) => secret_leaks += 1,
            Err(MemoryError::Secret(_)) => {}
            Err(error) => return Err(error).context("attempt a secret memory"),
        }
    }

    let mut supersession = SupersessionReport {
        count: 0,
        correct: 0,
    };
    for memory in &fixture.memories {
        let Some(old_key) = memory.supersedes_key.as_deref() else {
            continue;
        };
        supersession.count += 1;
        let old_id = key_to_id
            .get(old_key)
            .with_context(|| format!("missing old key {old_key}"))?;
        let new_id = key_to_id
            .get(&memory.key)
            .with_context(|| format!("missing new key {}", memory.key))?;
        let old = service.get(old_id).context("read superseded memory")?;
        let new = service.get(new_id).context("read replacement memory")?;
        if old.is_some_and(|item| item.status == MemoryStatus::Superseded)
            && new.is_some_and(|item| item.status == MemoryStatus::Active)
        {
            supersession.correct += 1;
        }
    }

    let mut overall = Acc::default();
    let mut categories: BTreeMap<String, Acc> = BTreeMap::new();
    let mut temporal = TemporalReport {
        count: 0,
        correct: 0,
    };
    let mut spot_check = 0usize;

    for question in &fixture.questions {
        if question.spot_check {
            spot_check += 1;
        }
        if question.category == "temporal" {
            temporal.count += 1;
            let at = question.as_of_micros.unwrap_or(FIXED_NOW);
            let items = service
                .as_of(Some(&question.question), at)
                .with_context(|| format!("as_of {}", question.key))?;
            if items
                .iter()
                .any(|item| has_any(&item.id, &question.expected_keys, &id_to_keys))
            {
                temporal.correct += 1;
            }
            continue;
        }

        let started = Instant::now();
        let items = service
            .recall(&question.question)
            .with_context(|| format!("recall {}", question.key))?;
        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let tokens = estimate_tokens(&service.render_recall(&items)) as usize;
        let mut retrieved = items.clone();
        for item in service
            .standing()
            .with_context(|| format!("standing {}", question.key))?
            .items
        {
            if !retrieved.iter().any(|seen| seen.id == item.id) {
                retrieved.push(item);
            }
        }
        let top = retrieved.iter().take(10);
        let hit = top
            .clone()
            .any(|item| has_any(&item.id, &question.expected_keys, &id_to_keys));
        let denominator = question.relevant_keys.len().clamp(1, 5);
        let relevant = retrieved
            .iter()
            .take(5)
            .filter(|item| has_any(&item.id, &question.relevant_keys, &id_to_keys))
            .count()
            .min(denominator);
        let precision = relevant as f64 / denominator as f64;

        overall.count += 1;
        overall.hits += usize::from(hit);
        overall.precision_sum += precision;
        overall.tokens.push(tokens);
        overall.latencies.push(latency_ms);

        let acc = categories.entry(question.category.clone()).or_default();
        acc.count += 1;
        acc.hits += usize::from(hit);
        acc.precision_sum += precision;
        acc.tokens.push(tokens);
        acc.latencies.push(latency_ms);
    }

    let categories: BTreeMap<String, CategoryReport> = categories
        .into_iter()
        .map(|(name, acc)| (name, acc.finish()))
        .collect();
    let paraphrase = categories.get("paraphrase").cloned();

    Ok(BenchReport {
        fixture_version: fixture.version,
        questions: fixture.questions.len(),
        spot_check,
        fixture_memories: fixture.memories.len(),
        stored_rows,
        recall_at_10: ratio(overall.hits, overall.count),
        precision_at_5: if overall.count == 0 {
            0.0
        } else {
            overall.precision_sum / overall.count as f64
        },
        recall_tokens_p95: p95_usize(&overall.tokens),
        retrieval_ms_p95: p95_f64(&overall.latencies),
        categories,
        paraphrase,
        temporal,
        supersession,
        duplicate_failures,
        secret_leaks,
        targets: BenchTargets {
            recall_at_10: TARGET_RECALL,
            precision_at_5: TARGET_PRECISION,
            recall_tokens_p95: TARGET_TOKENS,
            retrieval_ms_p95: TARGET_RETRIEVAL_MS,
            secret_leaks: 0,
        },
        scale: None,
    })
}

/// Seed a large synthetic library and measure retrieval latency.
pub fn run_scale(items: usize) -> Result<ScaleReport> {
    if items == 0 {
        bail!("scale item count must be positive");
    }
    let fixture: Fixture = serde_json::from_str(FIXTURE).context("parse the memory fixture")?;
    let dir = TempDir::new().context("create the scale home")?;
    let harness = Harness::open_with(dir.path(), Arc::new(FixedClock), Arc::new(UuidV7))
        .context("open the scale record")?;
    let session = harness.session().clone();
    let service = MemoryService::new(session.clone());
    let loaded = load_fixture(&service, &fixture)?;
    if items < loaded.stored_rows {
        bail!(
            "scale item count must be at least the {} fixture rows",
            loaded.stored_rows
        );
    }
    let noise = items - loaded.stored_rows;
    let now = FIXED_NOW;
    session
        .storage()
        .write(move |writer| -> Result<(), hearth_storage::StorageError> {
            for index in 0..noise {
                let id = format!("scale-{index:08}");
                let content = format!("scale fact {index} about the benchmark corpus");
                let fingerprint = format!("scale-fingerprint-{index:08}");
                writer.execute(
                    "INSERT INTO memory_items (id, kind, content, tags, fingerprint, importance, status, source_kind, standing, created_at) \
                     VALUES (?, 'fact', ?, '[]', ?, 50, 'active', 'session', 'none', ?)",
                    (id, content, fingerprint, now),
                )?;
            }
            Ok(())
        })
        .context("seed the scale memory library")?;

    let queries: Vec<&FixtureQuestion> = fixture
        .questions
        .iter()
        .filter(|question| question.category == "direct")
        .take(20)
        .collect();
    let mut matched = 0usize;
    for question in &queries {
        let found = service
            .search(&question.question, 5)
            .with_context(|| format!("scale search {}", question.key))?;
        if found
            .iter()
            .any(|item| has_any(&item.id, &question.expected_keys, &loaded.id_to_keys))
        {
            matched += 1;
        } else {
            bail!(
                "scale query {} did not match a fixture memory; the library has {} rows and the query returns {}",
                question.key,
                loaded.stored_rows + noise,
                found.len()
            );
        }
    }
    service.standing().context("warm the memory state")?;
    service
        .recall_read(&queries[0].question)
        .context("warm the retrieval path")?;
    let mut search_latencies = Vec::with_capacity(queries.len());
    let mut standing_latencies = Vec::with_capacity(queries.len());
    let mut latencies = Vec::with_capacity(queries.len());
    for question in &queries {
        let started = Instant::now();
        let _ = service
            .search(&question.question, 40)
            .with_context(|| format!("scale search {}", question.key))?;
        search_latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        let _ = service
            .standing()
            .with_context(|| format!("scale standing {}", question.key))?;
        standing_latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        let _ = service
            .recall_read(&question.question)
            .with_context(|| format!("scale recall {}", question.key))?;
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let mut real_latencies = Vec::with_capacity(queries.len());
    for question in &queries {
        let started = Instant::now();
        let _ = service
            .recall(&question.question)
            .with_context(|| format!("scale real recall {}", question.key))?;
        real_latencies.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(ScaleReport {
        items,
        fixture_rows: loaded.stored_rows,
        noise_rows: noise,
        queries: queries.len(),
        matched,
        search_ms_p95: p95_f64(&search_latencies),
        standing_ms_p95: p95_f64(&standing_latencies),
        retrieval_ms_p95: p95_f64(&latencies),
        real_recall_ms_p95: p95_f64(&real_latencies),
        target_ms: TARGET_RETRIEVAL_MS,
    })
}

fn has_any(id: &str, keys: &[String], id_to_keys: &BTreeMap<String, Vec<String>>) -> bool {
    id_to_keys
        .get(id)
        .is_some_and(|owned| owned.iter().any(|key| keys.contains(key)))
}

fn ratio(hits: usize, count: usize) -> f64 {
    if count == 0 {
        0.0
    } else {
        hits as f64 / count as f64
    }
}

fn p95_usize(values: &[usize]) -> usize {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[percentile_index(sorted.len())]
}

fn p95_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    sorted[percentile_index(sorted.len())]
}

fn percentile_index(len: usize) -> usize {
    let index = (len as f64 * 0.95).ceil() as usize;
    index.saturating_sub(1).min(len - 1)
}
