//! Reachability guard for the long-term memory store.
//!
//! Every stored memory must be reachable by a direct lookup and by the browse
//! path. A small fake embedder also proves that a vector-only paraphrase is
//! reachable without keyword overlap.

use std::sync::Arc;

use hearth_memory::{
    EmbeddingProvider, MemoryInput, MemorySearchOptions, MemoryStore, Result, SearchMode,
};
use tempfile::TempDir;

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
        if lower.contains("art") || lower.contains("paint") {
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

#[test]
fn every_stored_memory_is_reachable() {
    let directory = TempDir::new().expect("temp home");
    let store = MemoryStore::open(directory.path().join("long-term.sqlite")).expect("store");

    let mut ids = Vec::new();
    for index in 0..128 {
        let (memory, _) = store
            .add(&MemoryInput {
                content: format!("fact {index} {}", marker(index)),
                tags: Some(vec![format!("group-{}", index % 8)]),
                ..Default::default()
            })
            .expect("add");
        ids.push(memory.id);
    }
    store
        .add(&MemoryInput {
            content: "she was painting a sunrise".to_owned(),
            tags: Some(vec!["art".to_owned()]),
            ..Default::default()
        })
        .expect("art memory");

    for (index, id) in ids.iter().enumerate() {
        let response = store
            .search(
                &marker(index),
                SearchMode::Phrase,
                5,
                0,
                &MemorySearchOptions::default(),
            )
            .expect("search");
        assert!(
            response.results.iter().any(|hit| &hit.memory.id == id),
            "memory {index} was not reachable by its direct term"
        );
    }

    let mut browsed = Vec::new();
    let mut offset = 0;
    loop {
        let page = store
            .search(
                "",
                SearchMode::Any,
                50,
                offset,
                &MemorySearchOptions::default(),
            )
            .expect("browse");
        browsed.extend(page.results.into_iter().map(|hit| hit.memory.id));
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert_eq!(browsed.len(), 129, "browse lost a stored memory");

    store.set_embedder(Arc::new(FakeEmbedding));
    assert!(store.index_pending_vectors().unwrap() >= 129);
    let hybrid = store
        .search(
            "artwork",
            SearchMode::Any,
            5,
            0,
            &MemorySearchOptions::default(),
        )
        .expect("hybrid search");
    assert!(
        hybrid
            .results
            .iter()
            .any(|hit| hit.memory.content.contains("painting")),
        "vector-only paraphrase was not reachable"
    );
}
