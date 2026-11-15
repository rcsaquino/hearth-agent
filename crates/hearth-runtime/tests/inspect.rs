use hearth_core::{Harness, PromptInput, Save};
use hearth_memory::{Kind, MemoryService, NewMemory};
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
                rules: String::new(),
                rules_version: 0,
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
            rules: String::new(),
            rules_version: 0,
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
        .write(
            NewMemory::new("The user keeps the release checklist in docs/.")
                .kind(Kind::Rule)
                .pinned(true),
        )
        .expect("save memory");

    let records = list_memories(home.path(), 100).expect("list memories");
    assert_eq!(records.len(), 1);
    assert!(records[0].pinned, "a rule is shown as pinned");
    assert_eq!(records[0].kind, "rule");
    assert!(records[0].content.contains("release checklist"));
}

#[test]
fn a_replaced_rule_is_not_listed_as_pinned() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let memory = MemoryService::new(harness.session().clone());
    let rule = memory
        .write(
            NewMemory::new("Never deploy on a Friday.")
                .kind(Kind::Rule)
                .pinned(true),
        )
        .expect("rule")
        .memory()
        .clone();
    memory
        .write(
            NewMemory::new("Never deploy on a Friday.")
                .kind(Kind::Commitment)
                .supersedes(rule.id),
        )
        .expect("replacement");

    let records = list_memories(home.path(), 100).expect("list memories");
    assert_eq!(records.len(), 2);
    let retired = records
        .iter()
        .find(|record| record.kind == "rule")
        .expect("the rule row");
    assert!(!retired.pinned, "a replaced rule must not show as pinned");
    assert!(retired.retired, "the old row must show as retired");
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

#[test]
fn memory_stamps_follow_the_configured_zone() {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let settings = home.path().join("settings.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&settings)
        .expect("open settings");
    file.write_all(br#"{"identity":{"timezone":"Asia/Manila"}}"#)
        .expect("write settings");
    file.flush().expect("flush settings");

    let harness = Harness::open(home.path()).expect("open harness");
    let memory = MemoryService::new(harness.session().clone());
    memory
        .write(NewMemory::new("The release checklist lives in docs/."))
        .expect("save memory");

    let records = list_memories(home.path(), 100).expect("list memories");
    assert_eq!(records.len(), 1);
    let offset = if hearth_runtime::Zone::resolve("Asia/Manila").name() == "Asia/Manila" {
        "+08:00"
    } else {
        // This host has no Asia/Manila entry, so the zone falls back to UTC.
        "+00:00"
    };
    for stamp in [&records[0].created_at, &records[0].updated_at] {
        assert!(stamp.ends_with(offset), "{stamp} must end in {offset}");
        let bytes = stamp.as_bytes();
        assert_eq!(stamp.len(), 25, "not an ISO 8601 stamp: {stamp}");
        assert_eq!(bytes[4], b'-', "{stamp}");
        assert_eq!(bytes[7], b'-', "{stamp}");
        assert_eq!(bytes[10], b'T', "{stamp}");
        assert_eq!(bytes[13], b':', "{stamp}");
        assert_eq!(bytes[16], b':', "{stamp}");
    }
}

#[test]
fn latest_prompt_survives_redaction_of_an_ordinary_path() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    {
        let harness = Harness::open(home.path()).expect("open harness");
        let session = harness.session();
        let text = json!({
            "identity": "SOUL TEXT",
            "tools": [{"name": "files", "note": "spill/task-id.txt"}]
        })
        .to_string();
        session.save_prompt_blob(&text).expect("save prompt");
        let stored = session.latest_prompt().expect("latest").expect("blob");
        serde_json::from_str::<serde_json::Value>(&stored.text).expect("stored text still JSON");
    }
    let document = latest_prompt(home.path())
        .expect("inspect prompt")
        .expect("the prompt is readable");
    assert_eq!(document["identity"], "SOUL TEXT");
}
