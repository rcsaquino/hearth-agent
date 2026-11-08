use std::sync::Arc;

use hearth_core::{
    Clock, Harness, NewEntry, PromptInput, Save, UuidV7, plan_compaction, summary_content,
    summary_first_kept, summary_text,
};
use serde_json::json;
use tempfile::TempDir;

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_micros(&self) -> i64 {
        self.0
    }
}

fn harness_at(now: i64) -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open_with(home.path(), Arc::new(FixedClock(now)), Arc::new(UuidV7))
        .expect("open harness");
    (home, harness)
}

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

fn conversation(session: &hearth_core::Session) -> String {
    session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation")
        .id
}

fn save_entry(
    session: &hearth_core::Session,
    conversation_id: &str,
    kind: &str,
    text: &str,
) -> String {
    let entry = NewEntry {
        kind: kind.to_owned(),
        content: json!({"text": text}),
        source: None,
        prompt_hash: None,
    };
    session
        .save(Save::new(conversation_id).entry_with(entry))
        .expect("save entry")
        .entry_ids[0]
        .clone()
}

fn save_summary(
    session: &hearth_core::Session,
    conversation_id: &str,
    text: &str,
    first_seq: i64,
    last_seq: i64,
    first_kept: i64,
) -> String {
    let entry = NewEntry {
        kind: "summary".to_owned(),
        content: json!({
            "text": text,
            "covers": {"first_seq": first_seq, "last_seq": last_seq},
            "first_kept": first_kept,
        }),
        source: None,
        prompt_hash: None,
    };
    session
        .save(Save::new(conversation_id).entry_with(entry))
        .expect("save summary")
        .entry_ids[0]
        .clone()
}

fn prompt_input(conversation_id: &str) -> PromptInput {
    PromptInput {
        soul: "Test soul.".to_owned(),
        conversation_id: conversation_id.to_owned(),
        model: "model".to_owned(),
        tools: Vec::new(),
        standing: String::new(),
        standing_version: 0,
        recall: String::new(),
            skills_index: String::new(),
        runtime_note: "Runtime note.".to_owned(),
    }
}

#[test]
fn a_plan_covers_the_old_range_and_keeps_the_tail() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = conversation(session);
    for index in 0..6 {
        let kind = if index % 2 == 0 { "user" } else { "assistant" };
        save_entry(session, &conversation, kind, &"x".repeat(200));
    }
    let history = session.history(&conversation).expect("history");
    let tokens: Vec<u64> = history
        .iter()
        .map(|entry| entry.tokens_est as u64)
        .collect();
    let keep = tokens[4] + tokens[5];
    let plan = plan_compaction(&history, keep).expect("plan");
    assert_eq!(plan.entries.len(), 4);
    assert_eq!(plan.first_seq, history[0].seq);
    assert_eq!(plan.last_seq, history[3].seq);
    assert_eq!(plan.first_kept, history[4].seq);
}

#[test]
fn a_cut_never_keeps_a_tool_result_without_its_assistant() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = conversation(session);
    let user = save_entry(session, &conversation, "user", &"u".repeat(200));
    let assistant = save_entry(
        session,
        &conversation,
        "assistant",
        &serde_json::json!({"text": "a".repeat(200)}).to_string(),
    );
    let tool = save_entry(
        session,
        &conversation,
        "tool_result",
        &serde_json::json!({"type": "tool_result", "call_id": "c1", "content": "t".repeat(200)})
            .to_string(),
    );
    let later_user = save_entry(session, &conversation, "user", &"v".repeat(200));
    let later_assistant = save_entry(session, &conversation, "assistant", &"w".repeat(200));
    let _ = (user, later_user, later_assistant);
    let history = session.history(&conversation).expect("history");
    let tokens: Vec<u64> = history
        .iter()
        .map(|entry| entry.tokens_est as u64)
        .collect();
    let keep = tokens[2] + tokens[3] + tokens[4];
    let plan = plan_compaction(&history, keep).expect("plan");
    assert_eq!(
        plan.entries.len(),
        1,
        "only the first user should be covered"
    );
    assert_eq!(plan.first_kept, history[1].seq);
    assert_eq!(history[1].id, assistant);
    assert_eq!(history[2].id, tool);
}

#[test]
fn the_prompt_starts_with_the_summary_and_excludes_covered_entries() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = conversation(session);
    let _u1 = save_entry(session, &conversation, "user", "first");
    let _a1 = save_entry(session, &conversation, "assistant", "second");
    let u3 = save_entry(session, &conversation, "user", "third");
    let history = session.history(&conversation).expect("history");
    let u3_seq = history.iter().find(|entry| entry.id == u3).expect("u3").seq;
    let summary_id = save_summary(
        session,
        &conversation,
        "The first two entries are summarized.",
        history[0].seq,
        history[1].seq,
        u3_seq,
    );
    let visible = session.history_for_prompt(&conversation).expect("visible");
    assert_eq!(visible.len(), 2);
    assert_eq!(visible[0].id, summary_id);
    assert_eq!(visible[1].id, u3);
    assert_eq!(
        summary_text(&visible[0]).expect("summary text"),
        "The first two entries are summarized."
    );
    assert_eq!(summary_first_kept(&visible[0]), Some(u3_seq));

    let prompt = session
        .build_prompt(prompt_input(&conversation))
        .expect("prompt");
    let transcript = prompt.document["transcript"]
        .as_array()
        .expect("transcript");
    assert!(
        transcript.iter().any(|message| message["role"] == "system"
            && message["blocks"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("Summary of earlier conversation"))),
        "the summary message is missing: {transcript:?}"
    );
    assert!(
        !transcript
            .iter()
            .any(|message| message["blocks"][0]["text"] == "first"),
        "a covered entry leaked into the prompt"
    );
}

#[test]
fn a_second_compaction_can_cover_the_earlier_summary() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = conversation(session);
    let _u1 = save_entry(session, &conversation, "user", "first");
    let _a1 = save_entry(session, &conversation, "assistant", "second");
    let u3 = save_entry(session, &conversation, "user", "third");
    let history = session.history(&conversation).expect("history");
    let u3_seq = history.iter().find(|entry| entry.id == u3).expect("u3").seq;
    save_summary(
        session,
        &conversation,
        "First summary.",
        history[0].seq,
        history[1].seq,
        u3_seq,
    );
    let visible = session.history_for_prompt(&conversation).expect("visible");
    let plan = plan_compaction(&visible, 1).expect("plan");
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].kind, "summary");
    let content = summary_content("Second summary.", &plan);
    let entry = NewEntry {
        kind: "summary".to_owned(),
        content,
        source: None,
        prompt_hash: None,
    };
    session
        .save(Save::new(&conversation).entry_with(entry))
        .expect("second summary");
    let visible = session.history_for_prompt(&conversation).expect("visible");
    assert_eq!(visible.len(), 2);
    assert_eq!(summary_text(&visible[0]), Some("Second summary."));
    assert_eq!(visible[1].id, u3);
}

#[test]
fn a_reset_is_a_marker_and_is_not_sent() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = conversation(session);
    save_entry(session, &conversation, "user", "before");
    let reset = save_entry(session, &conversation, "reset", "");
    let after = save_entry(session, &conversation, "user", "after");
    let visible = session.history_for_prompt(&conversation).expect("visible");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, after);
    assert_ne!(visible[0].id, reset);
}

#[test]
fn idle_compaction_requires_idle_time_tokens_and_new_history() {
    let now = 1_800_000_000_000_000i64;
    let (_home, harness) = harness_at(now);
    let session = harness.session();
    let conversation = conversation(session);
    for index in 0..4 {
        let kind = if index % 2 == 0 { "user" } else { "assistant" };
        save_entry(session, &conversation, kind, &"x".repeat(500));
    }
    let candidate = session
        .idle_compaction_candidate(now, 3_600_000_000, 1)
        .expect("candidate lookup");
    assert!(candidate.is_none(), "a fresh conversation is not idle");

    let later = now + 3_600_000_001;
    let candidate = session
        .idle_compaction_candidate(later, 3_600_000_000, 1)
        .expect("candidate lookup");
    assert_eq!(
        candidate.map(|conversation| conversation.id),
        Some(conversation.clone()),
        "the idle conversation was not selected"
    );

    let history = session.history(&conversation).expect("history");
    save_summary(
        session,
        &conversation,
        "All old history.",
        history[0].seq,
        history[3].seq,
        history[3].seq + 1,
    );
    let after_summary = session
        .idle_compaction_candidate(later, 3_600_000_000, 1)
        .expect("candidate lookup");
    assert!(
        after_summary.is_none(),
        "a compacted history with no new entries was selected again"
    );

    save_entry(session, &conversation, "user", &"x".repeat(500));
    let after_new = session
        .idle_compaction_candidate(later, 3_600_000_000, 1)
        .expect("candidate lookup");
    assert!(
        after_new.is_some(),
        "new history after a summary was not selected"
    );
}
