use hearth_core::{Harness, PromptInput};
use tempfile::TempDir;

fn input(conversation_id: &str, skills_index: &str) -> PromptInput {
    PromptInput {
        soul: "SOUL".to_owned(),
        conversation_id: conversation_id.to_owned(),
        model: "test-model".to_owned(),
        tools: Vec::new(),
        standing: "PINNED FACT".to_owned(),
        standing_version: 1,
        recall: String::new(),
        skills_index: skills_index.to_owned(),
        guidance: String::new(),
        runtime_note: "RUNTIME NOTE".to_owned(),
    }
}

fn text(message: &hearth_model::ChatMessage) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            hearth_model::ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn the_skills_index_is_a_prompt_layer_after_standing() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");

    let prompt = harness
        .session()
        .build_prompt(input(&conversation.id, "- research: Research a topic"))
        .expect("prompt");

    let standing = prompt
        .request
        .messages
        .iter()
        .position(|message| text(message).contains("## Standing memory"))
        .expect("standing layer");
    let skills = prompt
        .request
        .messages
        .iter()
        .position(|message| text(message).contains("## Skills"))
        .expect("skills layer");
    let runtime = prompt
        .request
        .messages
        .iter()
        .position(|message| text(message).contains("RUNTIME NOTE"))
        .expect("runtime layer");
    assert!(standing < skills, "standing must precede skills");
    assert!(skills < runtime, "skills must precede runtime");
    assert!(
        text(&prompt.request.messages[skills]).contains("- research: Research a topic"),
        "skills text"
    );
    assert_eq!(
        prompt.document["skills_index"],
        "- research: Research a topic"
    );
    assert!(prompt.tokens.skills > 0, "skills token count");
    assert!(prompt.prefix_parts.contains_key("skills"));
}

#[test]
fn a_skills_index_change_changes_the_prefix() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");

    let first = harness
        .session()
        .build_prompt(input(&conversation.id, "- research: Research a topic"))
        .expect("first");
    let second = harness
        .session()
        .build_prompt(input(&conversation.id, "- writing: Write clearly"))
        .expect("second");

    assert_ne!(first.prefix_hash, second.prefix_hash);
    assert_ne!(
        first.prefix_parts.get("skills"),
        second.prefix_parts.get("skills")
    );
    let causes = hearth_core::prefix_cause(Some(&first.document), &second.prefix_parts);
    assert!(causes.contains(&"skills index".to_owned()), "{causes:?}");
}
