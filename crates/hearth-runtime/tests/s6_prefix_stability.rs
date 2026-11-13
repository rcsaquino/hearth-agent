use hearth_core::{Harness, PromptInput};
use hearth_memory::MemoryService;
use tempfile::TempDir;

fn input(conversation_id: &str, standing: String) -> PromptInput {
    PromptInput {
        soul: "SOUL".to_owned(),
        conversation_id: conversation_id.to_owned(),
        model: "test-model".to_owned(),
        tools: Vec::new(),
        standing,
        standing_version: 0,
        recall: String::new(),
        skills_index: String::new(),
        guidance: String::new(),
        runtime_note: "RUNTIME".to_owned(),
    }
}

#[test]
fn editing_a_standing_bullet_changes_the_prefix() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone()).expect("memory");
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    let added = memory
        .standing_store()
        .add("Use metric units", 90)
        .expect("first rule");
    let (before_text, before_version) = memory.standing().expect("standing");
    let before = session
        .build_prompt(input(&conversation.id, before_text))
        .expect("first prompt");

    // Changing the content of a bullet must invalidate the cached prefix.
    memory
        .standing_store()
        .edit(&added.memory.id, Some("Use metric units only"), None, None)
        .expect("edit");

    let (after_text, after_version) = memory.standing().expect("standing");
    assert_ne!(before_version, after_version);
    let after = session
        .build_prompt(input(&conversation.id, after_text))
        .expect("second prompt");

    assert_ne!(
        before.prefix_parts.get("standing"),
        after.prefix_parts.get("standing")
    );
    assert_ne!(before.prefix_hash, after.prefix_hash);
}

#[test]
fn reading_standing_memory_twice_leaves_the_prefix_unchanged() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone()).expect("memory");
    let conversation = session
        .create_conversation("Test", "local", "local:2")
        .expect("create conversation");

    memory
        .standing_store()
        .add("Use metric units", 90)
        .expect("first rule");
    memory
        .standing_store()
        .add("Use Celsius", 80)
        .expect("second rule");

    let (rendered, version) = memory.standing().expect("standing");
    let before = session
        .build_prompt(input(&conversation.id, rendered.clone()))
        .expect("first prompt");

    // A read must be pure: no archive, no rewrite, no reordering.
    let (again, again_version) = memory.standing().expect("standing again");
    assert_eq!(rendered, again);
    assert_eq!(version, again_version);
    let after = session
        .build_prompt(input(&conversation.id, again))
        .expect("second prompt");

    assert_eq!(
        before.prefix_parts.get("standing"),
        after.prefix_parts.get("standing")
    );
    assert_eq!(before.prefix_hash, after.prefix_hash);
}

#[test]
fn standing_bullets_keep_their_priority_order_across_rereads() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone()).expect("memory");

    memory
        .standing_store()
        .add("Lower priority rule", 60)
        .expect("low");
    memory
        .standing_store()
        .add("Higher priority rule", 95)
        .expect("high");

    let order = || {
        memory
            .standing_store()
            .entries()
            .expect("entries")
            .into_iter()
            .map(|item| (item.content, item.priority))
            .collect::<Vec<_>>()
    };

    // The file keeps insertion order; priority decides eviction, not layout.
    let before = order();
    assert_eq!(before.len(), 2);
    assert_eq!(order(), before);
}
