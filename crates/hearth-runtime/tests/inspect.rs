use hearth_core::{Harness, NewEntry, PromptInput, Save};
use hearth_memory::{Kind, MemoryService, NewMemory};
use hearth_runtime::inspect::{latest_prompt, list_memories, prompt_exchanges};
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
fn list_memories_groups_pinned_active_then_inactive() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let memory = MemoryService::new(harness.session().clone());
    let pinned = memory
        .write(NewMemory::new("Pinned rule.").kind(Kind::Rule).pinned(true))
        .expect("rule")
        .memory()
        .clone();
    let active = memory
        .write(NewMemory::new("Active fact.").subject("active"))
        .expect("active")
        .memory()
        .clone();
    let retired = memory
        .write(NewMemory::new("Retired fact.").subject("retired"))
        .expect("retired")
        .memory()
        .clone();
    memory
        .write(
            NewMemory::new("Replacement fact.")
                .subject("retired")
                .supersedes(retired.id.clone()),
        )
        .expect("replacement");

    let records = list_memories(home.path(), 100).expect("list memories");
    let at = |id: &str| {
        records
            .iter()
            .position(|record| record.id == id)
            .unwrap_or_else(|| panic!("missing {id}"))
    };
    assert!(
        at(&pinned.id) < at(&active.id),
        "a pinned row leads the list"
    );
    assert!(
        at(&active.id) < at(&retired.id),
        "an active row must not sit below a retired one"
    );
    assert_eq!(
        records.last().expect("last row").id,
        retired.id,
        "a retired row is history and sits last"
    );
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

#[test]
fn prompt_exchanges_settle_each_request_with_its_outcome() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Prompt", "local", "local:outcome")
        .expect("conversation");
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

    let answered = session.save_prompt_blob(&text).expect("answered hash");
    session
        .save(Save::new(&conversation.id).entry_with(NewEntry {
            kind: "assistant".to_owned(),
            content: json!([{"type": "text", "text": "the answer"}]),
            source: None,
            prompt_hash: Some(answered.clone()),
        }))
        .expect("answer");

    let failed = session
        .save_prompt_blob(&format!("{text} "))
        .expect("failed hash");
    session
        .save(Save::new(&conversation.id).entry_with(NewEntry {
            kind: "notice".to_owned(),
            content: json!({"text": "Provider error: boom", "reason": "provider_error"}),
            source: None,
            prompt_hash: Some(failed.clone()),
        }))
        .expect("notice");

    let running = session
        .save_prompt_blob(&format!("{text}  "))
        .expect("running hash");

    let exchanges = prompt_exchanges(home.path(), 10).expect("exchanges");
    assert_eq!(exchanges.len(), 3);
    let find = |hash: &str| {
        exchanges
            .iter()
            .find(|exchange| exchange["prompt_hash"] == hash)
            .unwrap_or_else(|| panic!("missing exchange {hash}"))
    };
    assert!(
        find(&answered)["response"].is_object(),
        "an answered request carries its reply"
    );
    assert!(find(&answered)["notice"].is_null());
    assert!(find(&failed)["response"].is_null());
    assert_eq!(
        find(&failed)["notice"]["reason"],
        "provider_error",
        "a failed request carries the reason it settled with"
    );
    assert!(find(&running)["response"].is_null());
    assert!(
        find(&running)["notice"].is_null(),
        "a request with no outcome yet says nothing"
    );
}

#[test]
fn memory_state_counts_the_store_not_the_loaded_page() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let memory = MemoryService::new(harness.session().clone());
    memory
        .write(NewMemory::new("Pinned rule.").kind(Kind::Rule).pinned(true))
        .expect("rule");
    let retired = memory
        .write(NewMemory::new("Retired fact.").subject("retired"))
        .expect("retired")
        .memory()
        .clone();
    memory
        .write(
            NewMemory::new("Replacement fact.")
                .subject("retired")
                .supersedes(retired.id),
        )
        .expect("replacement");
    let rows: Vec<NewMemory> = (0..1_005)
        .map(|index| NewMemory::new(format!("note {index}")).subject(format!("subject{index}")))
        .collect();
    memory.write_many(rows).expect("bulk write");

    let page = list_memories(home.path(), 1_000).expect("page");
    assert_eq!(page.len(), 1_000, "the tab page is bounded");
    let state = hearth_runtime::inspect::memory_state(home.path()).expect("memory state");
    assert_eq!(state.counts.pinned, 1, "{state:?}");
    assert_eq!(state.counts.active, 1_006, "{state:?}");
    assert_eq!(state.counts.retired, 1, "{state:?}");
    assert!(state.audit.is_none(), "no pass has run");
}

#[test]
fn memory_state_reads_the_last_audit_report() {
    let home = TempDir::new().expect("temp home");
    init(home.path(), options()).expect("init");
    let harness = Harness::open(home.path()).expect("open harness");
    let memory = MemoryService::new(harness.session().clone());
    let now = harness.session().now_micros();
    let content = "the user prefers dark mode";
    let fingerprint = hearth_memory::fingerprint(content);
    harness
        .session()
        .storage()
        .write(move |writer| {
            for (id, kind) in [("m_dup1", "fact"), ("m_dup2", "preference")] {
                writer.execute(
                    "INSERT INTO memories (id, kind, content, subject, terms, fingerprint, pinned, \
                     source_kind, source_entry, valid_from, valid_to, created_at, updated_at, revision) \
                     VALUES (?, ?, ?, 'user', '', ?, 0, 'user', NULL, ?, NULL, ?, ?, 1)",
                    hearth_storage::rusqlite::params![id, kind, content, fingerprint, now, now, now],
                )?;
            }
            Ok::<_, hearth_memory::MemoryError>(())
        })
        .expect("seed duplicates");
    memory.consolidate().expect("consolidate");

    let state = hearth_runtime::inspect::memory_state(home.path()).expect("memory state");
    let audit = state.audit.expect("the report is readable");
    assert_eq!(audit.canonical_closed, 1);
    assert_eq!(state.counts.active, 1);
    assert_eq!(state.counts.retired, 1);
}
