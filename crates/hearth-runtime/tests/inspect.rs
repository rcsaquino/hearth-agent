use hearth_core::{Harness, PromptInput, Save};
use hearth_memory::{MemoryKind, MemoryService, NewMemoryInput, SourceKind};
use hearth_runtime::inspect::{latest_prompt, list_memories};
use hearth_runtime::{InitOptions, init};
use serde_json::json;
use tempfile::TempDir;

fn options() -> InitOptions {
    InitOptions {
        telegram_token: "123:fake-token".to_owned(),
        allowed_user_ids: vec![42],
        base_url: "https://example.com/v1".to_owned(),
        api_key: "fake-key".to_owned(),
        model: "fake-model".to_owned(),
        thinking: "default".to_owned(),
        agent_name: "Hearth".to_owned(),
        timezone: Some("UTC".to_owned()),
        media: None,
        context_window: None,
        daemon: Default::default(),
    }
}

#[test]
fn latest_prompt_reads_the_newest_saved_document() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    {
        let harness = Harness::open(home.path()).expect("open harness");
        let session = harness.session();
        let conversation = session
            .create_conversation("Prompt", "local", "local:prompt")
            .expect("conversation");
        session
            .save(Save::new(&conversation.id).entry("user", json!({"text": "hello"})))
            .expect("user entry");
        let prompt = session
            .build_prompt(PromptInput {
                soul: "SOUL TEXT".to_owned(),
                conversation_id: conversation.id.clone(),
                model: "fake-model".to_owned(),
                tools: Vec::new(),
                standing: String::new(),
                standing_version: 0,
                recall: String::new(),
                skills_index: String::new(),
                guidance: String::new(),
                runtime_note: "RUNTIME".to_owned(),
            })
            .expect("prompt");
        let text = serde_json::to_string(&prompt.document).expect("prompt json");
        session.save_prompt_blob(&text).expect("save prompt");
    }

    let document = latest_prompt(home.path())
        .expect("inspect prompt")
        .expect("a prompt exists");
    assert_eq!(document["identity"], "SOUL TEXT");
    assert_eq!(document["model"], "fake-model");
}

#[test]
fn latest_prompt_is_none_without_a_database() {
    let home = TempDir::new().expect("temp home");
    assert!(latest_prompt(home.path()).expect("inspect").is_none());
}

#[test]
fn latest_prompt_is_visible_while_the_harness_keeps_the_database_open() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Prompt", "local", "local:live-prompt")
        .expect("conversation");
    let prompt = session
        .build_prompt(PromptInput {
            soul: "LIVE SOUL".to_owned(),
            conversation_id: conversation.id.clone(),
            model: "fake-model".to_owned(),
            tools: Vec::new(),
            standing: String::new(),
            standing_version: 0,
            recall: String::new(),
            skills_index: String::new(),
            guidance: String::new(),
            runtime_note: "RUNTIME".to_owned(),
        })
        .expect("prompt");
    let text = serde_json::to_string(&prompt.document).expect("prompt json");
    session.save_prompt_blob(&text).expect("save prompt");

    let document = latest_prompt(home.path())
        .expect("inspect prompt")
        .expect("a prompt exists while the harness is open");
    assert_eq!(document["identity"], "LIVE SOUL");
}

#[test]
fn list_memories_reads_saved_items_without_a_writer() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let memory = MemoryService::new(harness.session().clone());
    memory
        .save(NewMemoryInput {
            content: "The user keeps the release checklist in docs/.".to_owned(),
            kind: MemoryKind::Fact,
            tags: vec!["release".to_owned()],
            importance: 80,
            source_kind: SourceKind::User,
            source_ref: None,
            supersedes: None,
            valid_from: None,
            valid_to: None,
        })
        .expect("save memory");

    let records = list_memories(home.path(), 100).expect("list memories");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, "fact");
    assert_eq!(records[0].status, "active");
    assert!(records[0].content.contains("release checklist"));
    assert_eq!(records[0].tags, vec!["release"]);
}

#[test]
fn latest_prompt_skips_plain_text_maintenance_prompts() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    session
        .save_prompt_blob(r#"{"identity":"SOUL TEXT","model":"fake-model"}"#)
        .expect("save audit prompt");
    session
        .save_prompt_blob("Summarize these older messages in plain text.")
        .expect("save maintenance prompt");

    let document = latest_prompt(home.path())
        .expect("inspect prompt")
        .expect("the JSON prompt is still visible");
    assert_eq!(document["identity"], "SOUL TEXT");
}
