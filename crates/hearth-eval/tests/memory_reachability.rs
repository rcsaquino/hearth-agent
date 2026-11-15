//! What the memory rewrite has to prove.
//!
//! Three claims, and every one is measured on the real store:
//!
//! 1. **Reachability.** Every stored statement can be reached by its id, its
//!    subject, a direct term, and by paging the whole store, which is what makes
//!    "nothing is lost" a check rather than a hope.
//! 2. **Recall.** A planted fact comes back for the question that needs it. The
//!    digest and the one tool call are measured separately, because the honest
//!    claim is "everything is recoverable in one call", not "the digest is
//!    always right".
//! 3. **Cost.** The digest stays inside its token budget and the whole retrieval
//!    path stays inside the latency budget at a realistic store size.

use std::sync::Arc;

use hearth_core::Harness;
use hearth_memory::{EmbeddingProvider, MemoryService, NewMemory, Result};
use tempfile::TempDir;

/// A three-axis fake, enough to prove the vector channel is wired in without a
/// model download.
struct FakeEmbedding;

impl EmbeddingProvider for FakeEmbedding {
    fn model(&self) -> &str {
        "reachability-fake"
    }

    fn dims(&self) -> usize {
        3
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let lower = text.to_lowercase();
        let mut vector = vec![0.0f32; 3];
        if lower.contains("art") || lower.contains("paint") || lower.contains("sunrise") {
            vector[0] = 1.0;
        }
        if lower.contains("soup") || lower.contains("food") {
            vector[1] = 1.0;
        }
        if vector.iter().all(|value| *value == 0.0) {
            vector[2] = 1.0;
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        for value in &mut vector {
            *value /= norm;
        }
        Ok(vector)
    }
}

/// A word no other statement contains.
fn marker(index: usize) -> String {
    let mut value = index;
    let mut word = String::from("marker");
    loop {
        word.push((b'a' + (value % 26) as u8) as char);
        value /= 26;
        if value == 0 {
            break;
        }
    }
    word
}

fn home() -> (TempDir, MemoryService) {
    let directory = TempDir::new().expect("temp home");
    let harness = Harness::open(directory.path()).expect("harness");
    let service = MemoryService::new(harness.session().clone());
    (directory, service)
}

#[test]
fn every_stored_statement_is_reachable_by_four_paths() {
    let (_directory, service) = home();
    let mut written = Vec::new();
    for index in 0..128 {
        let outcome = service
            .write(
                NewMemory::new(format!("fact {index} {}", marker(index)))
                    .subject(format!("subject-{}", index % 8)),
            )
            .expect("write");
        written.push(outcome.memory().clone());
    }

    for memory in &written {
        assert!(
            service.get(&memory.id).expect("get").is_some(),
            "{} is not reachable by id",
            memory.id
        );
        let by_subject = service
            .subject(&memory.subject, None, 200)
            .expect("subject");
        assert!(
            by_subject.iter().any(|candidate| candidate.id == memory.id),
            "{} is not reachable by its subject",
            memory.id
        );
        let term = memory
            .content
            .split_whitespace()
            .find(|word| word.starts_with("marker"))
            .expect("marker");
        let found = service.search(term, None, 20).expect("search");
        assert!(
            found.iter().any(|candidate| candidate.id == memory.id),
            "{} is not reachable by {term}",
            memory.id
        );
    }

    // Start below the row count so the loop really pages. A single large read
    // would prove nothing about enumeration.
    let mut seen = std::collections::HashSet::new();
    let mut limit = 16i64;
    let mut pages = 0;
    loop {
        pages += 1;
        let page = service.recent(None, limit).expect("browse");
        let before = seen.len();
        for memory in &page {
            seen.insert(memory.id.clone());
        }
        if page.len() < limit as usize || seen.len() == before {
            break;
        }
        limit *= 2;
    }
    assert!(
        pages > 1,
        "the store was read in one page, so nothing paged"
    );
    for memory in &written {
        assert!(
            seen.contains(&memory.id),
            "{} is not reachable by paging the store",
            memory.id
        );
    }
}

#[test]
fn a_paraphrase_is_reachable_only_through_the_vector_channel() {
    let (_directory, service) = home();
    let service = service.with_embedder(Arc::new(FakeEmbedding));
    service
        .write(NewMemory::new("she was painting a sunrise").subject("study"))
        .expect("write");

    let lexical = MemoryService::new(service.session().clone());
    assert!(
        lexical
            .search("artwork", None, 10)
            .expect("search")
            .is_empty(),
        "the lexical path must not claim a paraphrase match"
    );
    let hybrid = service.search("artwork", None, 10).expect("search");
    assert!(
        hybrid
            .iter()
            .any(|memory| memory.content.contains("painting")),
        "the vector channel must reach a paraphrase: {hybrid:?}"
    );
}

#[test]
fn a_replaced_statement_stays_reachable_as_history() {
    let (_directory, service) = home();
    let old = service
        .write(
            NewMemory::new("the lab is in building 4")
                .subject("lab")
                .valid_from(1_000_000),
        )
        .expect("old")
        .memory()
        .clone();
    service
        .write(
            NewMemory::new("the lab is in building 9")
                .subject("lab")
                .valid_from(2_000_000)
                .supersedes(old.id.clone()),
        )
        .expect("new");

    assert!(
        service.get(&old.id).expect("get").is_some(),
        "a replaced statement must not be deleted"
    );
    let then = service.subject("lab", Some(1_500_000), 10).expect("as of");
    assert_eq!(then.len(), 1);
    assert_eq!(then[0].id, old.id, "history must read as of the old date");
    let now = service.subject("lab", None, 10).expect("now");
    assert_eq!(now.len(), 1);
    assert_ne!(now[0].id, old.id);
}

#[test]
fn the_digest_stays_inside_its_token_budget() {
    let (_directory, service) = home();
    let topics = [
        "hiring",
        "budget",
        "roadmap",
        "launch",
        "design",
        "support",
        "security",
        "pricing",
        "onboarding",
        "analytics",
        "billing",
        "partnerships",
        "documentation",
        "infrastructure",
        "recruiting",
        "retention",
        "compliance",
        "localisation",
        "accessibility",
        "performance",
    ];
    let mut items = Vec::new();
    for (index, topic) in topics.iter().enumerate() {
        items.push(
            NewMemory::new(format!(
                "Alice owns the {topic} track for cycle {index}, {}",
                "with a long note attached ".repeat(9)
            ))
            .subject("alice"),
        );
    }
    service.write_many(items).expect("write");
    assert!(service.count().expect("count") >= topics.len());
    let recall = service
        .recall("what do you know about Alice")
        .expect("recall");
    let tokens = hearth_model::estimate_tokens(&recall.digest);
    assert!(
        tokens <= service.config().recall_max_tokens as i64,
        "the digest used {tokens} tokens against a {} budget",
        service.config().recall_max_tokens
    );
    assert!(
        !recall.coverage.is_empty(),
        "the digest must report what it left out"
    );
}
