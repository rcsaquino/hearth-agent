use hearth_core::{Harness, PromptInput};
use hearth_memory::{MemoryService, NewMemory};
use tempfile::TempDir;

fn input(conversation_id: &str, rules: String) -> PromptInput {
    PromptInput {
        soul: "SOUL".to_owned(),
        conversation_id: conversation_id.to_owned(),
        model: "test-model".to_owned(),
        tools: Vec::new(),
        rules,
        rules_version: 0,
        recall: String::new(),
        skills_index: String::new(),
        guidance: String::new(),
        runtime_note: "RUNTIME".to_owned(),
    }
}

fn rule(memory: &MemoryService, content: &str) -> hearth_memory::Memory {
    memory
        .write(
            NewMemory::new(content)
                .kind(hearth_memory::Kind::Rule)
                .pinned(true),
        )
        .expect("rule")
        .memory()
        .clone()
}

#[test]
fn replacing_a_rule_changes_the_prefix() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone());
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("create conversation");

    rule(&memory, "Use metric units");
    let before_rules = memory.rules().expect("rules");
    let before = session
        .build_prompt(input(&conversation.id, before_rules.text.clone()))
        .expect("first prompt");

    // Replacing the rule must invalidate the cached prefix.
    let first = memory.recent(None, 10).expect("rules");
    let first_id = first
        .iter()
        .find(|memory| memory.content == "Use metric units")
        .expect("the rule row")
        .id
        .clone();
    memory
        .write(
            NewMemory::new("Use metric units and Celsius only")
                .kind(hearth_memory::Kind::Rule)
                .pinned(true)
                .supersedes(first_id),
        )
        .expect("replacement rule");

    let after_rules = memory.rules().expect("rules");
    assert_ne!(before_rules.version, after_rules.version);
    assert_ne!(before_rules.text, after_rules.text);
    let after = session
        .build_prompt(input(&conversation.id, after_rules.text))
        .expect("second prompt");

    assert_ne!(
        before.prefix_parts.get("rules"),
        after.prefix_parts.get("rules")
    );
    assert_ne!(before.prefix_hash, after.prefix_hash);
}

#[test]
fn reading_rules_twice_leaves_the_prefix_unchanged() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone());
    let conversation = session
        .create_conversation("Test", "local", "local:2")
        .expect("create conversation");

    rule(&memory, "Use metric units");
    rule(&memory, "Use Celsius");

    let before_rules = memory.rules().expect("rules");
    let before = session
        .build_prompt(input(&conversation.id, before_rules.text.clone()))
        .expect("first prompt");

    // A read must be pure: no write, no reordering.
    let again = memory.rules().expect("rules again");
    assert_eq!(before_rules.text, again.text);
    assert_eq!(before_rules.version, again.version);
    let after = session
        .build_prompt(input(&conversation.id, again.text))
        .expect("second prompt");

    assert_eq!(
        before.prefix_parts.get("rules"),
        after.prefix_parts.get("rules")
    );
    assert_eq!(before.prefix_hash, after.prefix_hash);
}

#[test]
fn rules_keep_their_creation_order_across_rereads() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let memory = MemoryService::new(harness.session().clone());

    rule(&memory, "Low priority rule");
    rule(&memory, "High priority rule");

    let before = memory.rules().expect("rules").text;
    let lines: Vec<&str> = before.lines().collect();
    assert_eq!(lines.len(), 2, "{before}");
    assert!(
        lines[0].ends_with("Low priority rule"),
        "a rule keeps the position it was created in: {before}"
    );
    assert!(lines[1].ends_with("High priority rule"), "{before}");
    assert_eq!(memory.rules().expect("rules").text, before);
}
