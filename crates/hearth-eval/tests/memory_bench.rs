//! The memory benchmark.
//!
//! The fixture plants two hundred statements and asks two hundred questions about
//! them. The benchmark reports two different numbers on purpose:
//!
//! * **Automatic recall** is what the digest put in front of the model. It is what
//!   the model gets for free.
//! * **Reachability** is automatic recall, or the expected statement appearing in
//!   one `memory_recall` call. It is the honest form of "nothing is lost": a miss
//!   in the digest is recoverable in one turn.
//!
//! Both are printed with the category they came from, so a regression names the
//! thing that broke instead of moving an average.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use hearth_core::Harness;
use hearth_memory::{Kind, MemoryService, NewMemory, Source};
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;

const FIXTURE: &str = include_str!("../fixtures/memory_bench.json");
/// The digest is allowed to cost this much at the 95th percentile.
const TARGET_TOKENS: u64 = 300;
/// Retrieval may take this long at the 95th percentile on a personal store.
///
/// The measured store is the 200-statement fixture plus 4,000 distractors. The
/// vector scan is linear from about 10 ms at 4,000 rows, and a 200,000-row store
/// has not been measured since the retrieval rewrite. SPEC 7.9 and 14 quote this
/// number, so it is the one gate.
const TARGET_MS: f64 = 100.0;
/// The digest should carry the answer for this share of questions.
///
/// Six of the thirteen misses sit on a topic the fixture gives two live answers
/// to, and the rest are paraphrases the lexical path cannot reach. The gate is one
/// point under the measured value so a real regression fails it and fixture noise
/// does not.
const TARGET_AUTOMATIC: f64 = 0.92;
/// And every planted statement must be recoverable in one call.
const TARGET_REACHABLE: f64 = 0.98;

#[derive(Debug, Deserialize)]
struct Fixture {
    memories: Vec<FixtureMemory>,
    questions: Vec<FixtureQuestion>,
}

#[derive(Debug, Deserialize)]
struct FixtureMemory {
    key: String,
    content: String,
    kind: String,
    #[serde(default)]
    subject: String,
    valid_from: Option<i64>,
    valid_to: Option<i64>,
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

/// One category's numbers.
#[derive(Debug, Clone, Default)]
struct CategoryReport {
    count: usize,
    automatic_hits: usize,
    reachable_hits: usize,
}

impl CategoryReport {
    fn automatic(&self) -> f64 {
        ratio(self.automatic_hits, self.count)
    }

    fn reachable(&self) -> f64 {
        ratio(self.reachable_hits, self.count)
    }
}

#[derive(Debug, Clone)]
struct Report {
    questions: usize,
    fixture_memories: usize,
    stored_rows: usize,
    spot_check: usize,
    automatic: f64,
    reachable: f64,
    precision_at_5: f64,
    tokens_p95: u64,
    latency_p50: f64,
    latency_p95: f64,
    latency_p99: f64,
    categories: BTreeMap<String, CategoryReport>,
    duplicates_refused: usize,
    duplicate_failures: usize,
    secret_leaks: usize,
    supersession_correct: usize,
    supersession_count: usize,
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() as f64 - 1.0) * fraction).round() as usize;
    values[index]
}

/// The subject the fixture records for a statement.
///
/// It is committed data, not a value the product computes at run time. If it came
/// from `suggest_subject`, a regression in that rule would silently change the
/// index the questions are scored against and the benchmark would still pass.
fn subject_of(memory: &FixtureMemory) -> String {
    memory.subject.clone()
}

fn load() -> Fixture {
    serde_json::from_str(FIXTURE).expect("the fixture parses")
}

/// Credential shapes built at run time.
///
/// The repository holds no literal credential, and the fixture's `<...>` entries
/// are placeholders that must not be counted as leaks. These are the real shapes
/// the writer has to refuse.
fn adversarial_secrets() -> Vec<String> {
    let bytes = |count: usize| {
        "abcdefghijklmnopqrstuvwxyz0123456789"
            .chars()
            .cycle()
            .take(count)
            .collect::<String>()
    };
    vec![
        format!("sk-{}", bytes(32)),
        format!("ghp_{}", bytes(36)),
        format!("xoxb-{}", bytes(20)),
        format!("AKIA{}", bytes(16).to_uppercase()),
        format!("{}:{}", "123456789", bytes(35)),
        format!("api_key={}", bytes(16)),
        format!("password={}", bytes(14)),
        format!("token={}", bytes(15)),
        format!("secret={}", bytes(15)),
        format!("Bearer {}", bytes(18)),
    ]
}

/// Seed the fixture exactly as it reads.
///
/// The fixture is a coherent history: a statement carries the window it was true
/// for, and a topic's statements are consecutive. Replaying it verbatim is the
/// only seeding that does not smuggle the benchmark's own opinion into the store.
fn seed(service: &MemoryService, fixture: &Fixture) -> HashMap<String, String> {
    let mut batch = Vec::with_capacity(fixture.memories.len());
    for memory in &fixture.memories {
        let mut item = NewMemory::new(memory.content.clone())
            .source(Source::Import)
            .subject(subject_of(memory));
        if let Ok(kind) = memory.kind.parse::<Kind>() {
            item = item.kind(kind);
        }
        if let Some(from) = memory.valid_from {
            item = item.valid_from(from);
        }
        item = item.valid_to(memory.valid_to);
        batch.push((memory.key.clone(), item));
    }
    let items: Vec<NewMemory> = batch.iter().map(|(_, item)| item.clone()).collect();
    let outcomes = service.write_many(items).expect("seed the store");
    let mut by_key = HashMap::new();
    for ((key, _), outcome) in batch.iter().zip(outcomes.iter()) {
        by_key.insert(key.clone(), outcome.memory().id.clone());
    }
    by_key
}

fn run_fixture(service: &MemoryService, fixture: &Fixture) -> Report {
    let by_key = seed(service, fixture);
    let by_id: HashMap<String, Vec<String>> =
        by_key
            .iter()
            .fold(HashMap::new(), |mut reverse, (key, id)| {
                reverse.entry(id.clone()).or_default().push(key.clone());
                reverse
            });
    for (id, keys) in &by_id {
        if keys.len() < 2 {
            continue;
        }
        // Merging an identical statement is the intended behaviour. Merging two
        // different statements is the failure that silently destroys a fact.
        let contents: HashSet<&str> = keys
            .iter()
            .filter_map(|key| {
                fixture
                    .memories
                    .iter()
                    .find(|memory| &memory.key == key)
                    .map(|memory| memory.content.as_str())
            })
            .collect();
        assert_eq!(
            contents.len(),
            1,
            "{id} merged statements that say different things: {keys:?}"
        );
    }

    let mut categories: BTreeMap<String, CategoryReport> = BTreeMap::new();
    let mut automatic_hits = 0usize;
    let mut reachable_hits = 0usize;
    let mut precision_sum = 0.0;
    let mut tokens = Vec::new();
    let mut latencies = Vec::new();
    let mut supersession_correct = 0usize;
    let mut supersession_count = 0usize;

    for question in &fixture.questions {
        let started = Instant::now();
        let recall = service
            .recall_at(&question.question, question.as_of_micros)
            .expect("recall");
        latencies.push(started.elapsed().as_secs_f64() * 1_000.0);
        tokens.push(hearth_model::estimate_tokens(&recall.digest).max(0) as u64);

        let shown: Vec<&str> = recall
            .items
            .iter()
            .take(5)
            .map(|memory| memory.id.as_str())
            .collect();
        let automatic = question.expected_keys.iter().all(|key| {
            by_key
                .get(key)
                .is_some_and(|id| shown.contains(&id.as_str()))
        });
        if !automatic && std::env::var("HEARTH_BENCH_VERBOSE").is_ok() {
            let expected: Vec<&str> = question
                .expected_keys
                .iter()
                .filter_map(|key| {
                    fixture
                        .memories
                        .iter()
                        .find(|memory| &memory.key == key)
                        .map(|memory| memory.content.as_str())
                })
                .collect();
            println!(
                "MISS {} [{}] {:?} -> {:?}\n     shown: {:?}",
                question.key,
                question.category,
                question.question,
                expected,
                recall
                    .items
                    .iter()
                    .take(5)
                    .filter_map(|memory| by_id.get(&memory.id))
                    .flatten()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        }
        let reachable = automatic || {
            let found = service
                .search(&question.question, question.as_of_micros, 20)
                .expect("search");
            let found: HashSet<&str> = found.iter().map(|memory| memory.id.as_str()).collect();
            question.expected_keys.iter().all(|key| {
                by_key
                    .get(key)
                    .is_some_and(|id| found.contains(id.as_str()))
            })
        };

        let relevant = question.relevant_keys.len().clamp(1, 5);
        let hits = question
            .relevant_keys
            .iter()
            .filter(|key| {
                by_key
                    .get(*key)
                    .is_some_and(|id| shown.contains(&id.as_str()))
            })
            .count();
        precision_sum += hits as f64 / relevant as f64;

        if question.category == "contradiction" {
            supersession_count += 1;
            // The falsifiable part: every closed statement sharing the expected
            // statement's subject must be absent from the digest. A live read
            // filters closed rows, so if the filter broke this would catch it.
            let subjects: Vec<String> = question
                .expected_keys
                .iter()
                .filter_map(|key| {
                    fixture
                        .memories
                        .iter()
                        .find(|memory| &memory.key == key)
                        .map(subject_of)
                })
                .collect();
            let closed: Vec<&str> = fixture
                .memories
                .iter()
                .filter(|memory| memory.valid_to.is_some())
                .filter(|memory| subjects.contains(&subject_of(memory)))
                .filter_map(|memory| by_key.get(&memory.key).map(String::as_str))
                .collect();
            let stale_shown = recall
                .items
                .iter()
                .any(|memory| closed.contains(&memory.id.as_str()));
            if automatic && !stale_shown {
                supersession_correct += 1;
            }
        }

        let entry = categories.entry(question.category.clone()).or_default();
        entry.count += 1;
        entry.automatic_hits += usize::from(automatic);
        entry.reachable_hits += usize::from(reachable);
        automatic_hits += usize::from(automatic);
        reachable_hits += usize::from(reachable);
    }

    let mut duplicates_refused = 0usize;
    let mut duplicate_failures = 0usize;
    for memory in fixture.memories.iter().filter(|m| m.duplicate_of.is_some()) {
        let outcome = service
            .write(
                NewMemory::new(memory.content.clone())
                    .source(Source::Import)
                    .subject(subject_of(memory)),
            )
            .expect("duplicate write");
        if outcome.created() {
            duplicate_failures += 1;
        } else {
            duplicates_refused += 1;
        }
    }

    let mut secret_leaks = 0usize;
    for (index, secret) in adversarial_secrets().iter().enumerate() {
        let outcome = service.write(
            NewMemory::new(format!("the credential for service {index} is {secret}"))
                .source(Source::Import),
        );
        if outcome.is_ok() {
            secret_leaks += 1;
        }
    }

    let stored_rows = service.count().expect("count");
    let questions = fixture.questions.len();
    Report {
        questions,
        fixture_memories: fixture.memories.len(),
        stored_rows,
        spot_check: fixture.questions.iter().filter(|q| q.spot_check).count(),
        automatic: ratio(automatic_hits, questions),
        reachable: ratio(reachable_hits, questions),
        precision_at_5: precision_sum / questions as f64,
        tokens_p95: {
            let mut values: Vec<f64> = tokens.iter().map(|value| *value as f64).collect();
            percentile(&mut values, 0.95) as u64
        },
        latency_p50: percentile(&mut latencies.clone(), 0.50),
        latency_p95: percentile(&mut latencies.clone(), 0.95),
        latency_p99: percentile(&mut latencies, 0.99),
        categories,
        duplicates_refused,
        duplicate_failures,
        secret_leaks,
        supersession_correct,
        supersession_count,
    }
}

fn print(report: &Report) {
    println!(
        "questions {} · memories {} · stored {} · spot-check {}",
        report.questions, report.fixture_memories, report.stored_rows, report.spot_check
    );
    println!(
        "automatic recall {:.3} (target {TARGET_AUTOMATIC:.2}) · recoverable in one call {:.3} (target {TARGET_REACHABLE:.2})",
        report.automatic, report.reachable
    );
    println!(
        "precision@5 {:.3} · digest tokens p95 {} (target {TARGET_TOKENS})",
        report.precision_at_5, report.tokens_p95
    );
    println!(
        "latency p50 {:.2} ms · p95 {:.2} ms · p99 {:.2} ms (target p95 {TARGET_MS:.0} ms)",
        report.latency_p50, report.latency_p95, report.latency_p99
    );
    println!(
        "supersession {}/{} · duplicates refused {} (failures {}) · secret leaks {}",
        report.supersession_correct,
        report.supersession_count,
        report.duplicates_refused,
        report.duplicate_failures,
        report.secret_leaks
    );
    for (category, entry) in &report.categories {
        println!(
            "  {category:<14} n={:<3} automatic {:.3} recoverable {:.3}",
            entry.count,
            entry.automatic(),
            entry.reachable()
        );
    }
}

fn service() -> (TempDir, MemoryService) {
    let directory = TempDir::new().expect("temp home");
    let harness = Harness::open(directory.path()).expect("harness");
    let service = MemoryService::new(harness.session().clone());
    (directory, service)
}

#[test]
fn the_memory_benchmark_meets_its_targets() {
    let fixture = load();
    let (_directory, service) = service();
    let report = run_fixture(&service, &fixture);
    print(&report);
    println!("{}", json!({"report": format!("{report:?}")}));

    assert_eq!(report.questions, 195, "the fixture asks 195 questions");
    assert_eq!(
        report.fixture_memories, 200,
        "the fixture plants 200 statements"
    );
    assert_eq!(
        report.spot_check, 20,
        "twenty questions are the spot-check set"
    );
    assert_eq!(report.secret_leaks, 0, "a credential reached the store");
    assert_eq!(
        report.supersession_count, 15,
        "the fixture must keep its fifteen contradiction questions, or the gate measures nothing"
    );
    assert_eq!(
        report.duplicates_refused + report.duplicate_failures,
        5,
        "the fixture must keep its five duplicate statements"
    );
    assert_eq!(report.duplicate_failures, 0, "a duplicate became a new row");
    assert_eq!(
        report.supersession_correct, report.supersession_count,
        "a replaced statement answered a current question"
    );
    assert!(
        report.reachable >= TARGET_REACHABLE,
        "recoverable in one call was {:.3}",
        report.reachable
    );
    assert!(
        report.automatic >= TARGET_AUTOMATIC,
        "automatic recall was {:.3}",
        report.automatic
    );
    assert!(
        report.tokens_p95 <= TARGET_TOKENS,
        "the digest cost {} tokens at p95",
        report.tokens_p95
    );
    assert!(
        report.latency_p95 <= TARGET_MS,
        "retrieval took {:.2} ms at p95",
        report.latency_p95
    );
}

#[test]
fn the_spot_check_labels_are_honest() {
    let fixture = load();
    let spot: Vec<&FixtureQuestion> = fixture
        .questions
        .iter()
        .filter(|question| question.spot_check)
        .collect();
    assert_eq!(spot.len(), 20, "twenty questions are the spot-check set");
    for question in spot {
        assert_eq!(
            question.relevant_keys, question.expected_keys,
            "{} labels unrelated statements as relevant",
            question.key
        );
    }
}

#[test]
fn a_noisy_store_keeps_the_answer_in_reach() {
    let fixture = load();
    let (_directory, service) = service();
    let mut items = Vec::new();
    for index in 0..4_000 {
        items.push(
            NewMemory::new(format!(
                "noise note {index} about a routine standup and a routine ticket"
            ))
            .subject(format!("noise-{}", index % 40)),
        );
    }
    service.write_many(items).expect("noise");
    let report = run_fixture(&service, &fixture);
    print(&report);
    assert!(
        report.stored_rows >= 4_000,
        "the noisy store did not hold the noise"
    );
    assert_eq!(
        report.duplicates_refused + report.duplicate_failures,
        5,
        "noise must not hide the duplicate population"
    );
    assert!(
        report.reachable >= TARGET_REACHABLE,
        "noise pushed a planted statement out of reach: {:.3}",
        report.reachable
    );
    assert!(
        report.latency_p95 <= TARGET_MS,
        "retrieval took {:.2} ms at p95 with 4k distractors",
        report.latency_p95
    );
}

/// The same fixture with the local encoder installed.
///
/// Ignored by default because it needs the model on disk. Run it with
/// `HEARTH_EMBEDDING_MODEL_DIR` set to the directory holding `model.onnx` and
/// `tokenizer.json`:
///
/// ```text
/// HEARTH_EMBEDDING_MODEL_DIR=~/.hearth/models/bekko-embedding-v1-a8m \
///   cargo test -p hearth-eval --test memory_bench the_hybrid -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a local embedding model"]
fn the_hybrid_path_answers_the_paraphrases() {
    let directory =
        std::env::var("HEARTH_EMBEDDING_MODEL_DIR").expect("set HEARTH_EMBEDDING_MODEL_DIR");
    let embedder = hearth_memory::BekkoEmbedding::open(&directory)
        .expect("open the model")
        .expect("the model files are present");
    let fixture = load();
    let (_home, base) = service();
    let service = base.clone().with_embedder(std::sync::Arc::new(embedder));
    // Backfill the vectors the fixture rows were written without.
    let indexed = service.index_pending_vectors().expect("backfill");
    println!("indexed {indexed} vectors");
    let report = run_fixture(&service, &fixture);
    print(&report);
    assert_eq!(report.secret_leaks, 0);
    assert!(
        report.reachable >= 0.98,
        "the hybrid path reached {:.3}",
        report.reachable
    );
    assert!(
        report.categories["paraphrase"].automatic() >= 0.85,
        "paraphrase automatic was {:.3}",
        report.categories["paraphrase"].automatic()
    );
}
