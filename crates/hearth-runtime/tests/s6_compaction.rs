use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use hearth_core::{
    ContextBudget, Harness, NewEntry, Save, Submission, plan_compaction, summary_text,
};
use hearth_memory::MemoryService;
use hearth_model::{
    ChatRequest, Price, Pricing, Provider, ProviderError, ProviderStream, StreamEvent, Usage,
};
use hearth_runtime::{Agent, AgentConfig, Compactor};
use hearth_tools::ToolRegistry;
use serde_json::{Value, json};
use tempfile::TempDir;

type Script = Result<Vec<Result<StreamEvent, ProviderError>>, ProviderError>;

struct ScriptedProvider {
    scripts: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    fn new(scripts: Vec<Script>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

impl Provider for ScriptedProvider {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().expect("requests").push(request);
        let script = self
            .scripts
            .lock()
            .expect("scripts")
            .pop_front()
            .expect("a scripted response");
        match script {
            Ok(events) => Ok(Box::new(events.into_iter())),
            Err(error) => Err(error),
        }
    }
}

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

fn conversation(harness: &Harness) -> String {
    harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("conversation")
        .id
}

fn save_entry(harness: &Harness, conversation: &str, kind: &str, content: Value) -> String {
    harness
        .session()
        .save(Save::new(conversation).entry_with(NewEntry {
            kind: kind.to_owned(),
            content,
            source: None,
            prompt_hash: None,
        }))
        .expect("save")
        .entry_ids[0]
        .clone()
}

fn long_entries(harness: &Harness, conversation: &str, count: usize) {
    for index in 0..count {
        let kind = if index % 2 == 0 { "user" } else { "assistant" };
        save_entry(
            harness,
            conversation,
            kind,
            json!({"text": "x".repeat(500)}),
        );
    }
}

fn submit(harness: &Harness, conversation: &str) -> Submission {
    harness
        .session()
        .submit(conversation, "req-1", "input", json!({"text": "hi"}))
        .expect("submit")
        .submission
}

fn summary_script(text: &str) -> Script {
    Ok(vec![
        Ok(StreamEvent::TextDelta(text.to_owned())),
        Ok(StreamEvent::Usage(Usage {
            input_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 10,
            reasoning_tokens: 0,
        })),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])
}

fn text_script(text: &str) -> Script {
    summary_script(text)
}

fn compactor(
    harness: &Harness,
    provider: Arc<dyn Provider>,
    memory: Option<Arc<MemoryService>>,
) -> Compactor {
    let budget = ContextBudget::default();
    Compactor::new(
        harness.session().clone(),
        provider,
        Pricing::default(),
        memory,
        "test-model".to_owned(),
        "scripted".to_owned(),
        "test-owner".to_owned(),
        budget.reserve,
        budget.max_output_tokens,
    )
}

fn agent(
    home: &TempDir,
    harness: &Harness,
    provider: Arc<dyn Provider>,
    budget: ContextBudget,
) -> Agent {
    Agent::new(
        harness.session().clone(),
        provider,
        Pricing::default(),
        AgentConfig {
            soul: "Test soul.".to_owned(),
            model: "test-model".to_owned(),
            provider_name: "scripted".to_owned(),
            owner: "test-owner".to_owned(),
            max_turns: 4,
            tools: ToolRegistry::default(),
            spill_dir: home.path().join("spill"),
            voice_tags: false,
            guidance: String::new(),
        },
    )
    .with_budget(budget)
}

#[test]
fn a_compaction_saves_a_summary_and_records_usage() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    long_entries(&harness, &conversation, 6);
    let history = harness.session().history(&conversation).expect("history");
    let plan = plan_compaction(&history, 300).expect("plan");
    let provider = Arc::new(ScriptedProvider::new(vec![summary_script(
        "Old range summary.",
    )]));
    let summary = compactor(&harness, provider.clone(), None)
        .compact(&conversation, &plan)
        .expect("compact")
        .expect("summary");
    assert_eq!(summary_text(&summary), Some("Old range summary."));
    assert_eq!(summary.content["first_kept"], plan.first_kept);
    assert_eq!(summary.content["covers"]["first_seq"], plan.first_seq);
    assert_eq!(summary.content["covers"]["last_seq"], plan.last_seq);
    let visible = harness
        .session()
        .history_for_prompt(&conversation)
        .expect("visible");
    assert_eq!(visible[0].id, summary.id);
    assert_eq!(visible.len(), 1 + history.len() - plan.entries.len());
    let usage = harness
        .session()
        .latest_usage()
        .expect("usage")
        .expect("usage row");
    assert_eq!(usage.kind, "compaction");
    let request = &provider.requests()[0];
    assert_eq!(request.max_tokens, Some(7372));
    let user = request.messages[1].blocks[0]
        .as_text()
        .expect("summary user text");
    assert!(user.contains("Conversation range"), "{user}");
}

trait TextOf {
    fn as_text(&self) -> Option<&str>;
}

impl TextOf for hearth_model::ContentBlock {
    fn as_text(&self) -> Option<&str> {
        match self {
            hearth_model::ContentBlock::Text(text) => Some(text),
            _ => None,
        }
    }
}

#[test]
fn the_summary_prompt_truncates_long_tool_results() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    let long = "y".repeat(2000);
    save_entry(
        &harness,
        &conversation,
        "user",
        json!({"text": "look at this"}),
    );
    save_entry(
        &harness,
        &conversation,
        "tool_result",
        json!([{"type": "tool_result", "call_id": "c1", "content": long}]),
    );
    for index in 0..4 {
        let kind = if index % 2 == 0 { "user" } else { "assistant" };
        save_entry(
            &harness,
            &conversation,
            kind,
            json!({"text": "z".repeat(500)}),
        );
    }
    let history = harness.session().history(&conversation).expect("history");
    let plan = plan_compaction(&history, 100).expect("plan");
    let provider = Arc::new(ScriptedProvider::new(vec![summary_script("Summary.")]));
    compactor(&harness, provider.clone(), None)
        .compact(&conversation, &plan)
        .expect("compact");
    let request = &provider.requests()[0];
    let user = request.messages[1].blocks[0]
        .as_text()
        .expect("summary user text");
    assert!(
        !user.contains(&"y".repeat(401)),
        "the full tool result leaked into the summary prompt"
    );
    assert!(user.contains(&"y".repeat(400)), "{user}");
}

#[test]
fn a_second_compaction_includes_the_earlier_summary() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    long_entries(&harness, &conversation, 6);
    let history = harness.session().history(&conversation).expect("history");
    let plan = plan_compaction(&history, 300).expect("first plan");
    let provider = Arc::new(ScriptedProvider::new(vec![
        summary_script("First summary."),
        summary_script("Second summary."),
    ]));
    let first = compactor(&harness, provider.clone(), None)
        .compact(&conversation, &plan)
        .expect("first compact")
        .expect("first summary");
    assert_eq!(summary_text(&first), Some("First summary."));
    let visible = harness
        .session()
        .history_for_prompt(&conversation)
        .expect("visible");
    let second_plan = plan_compaction(&visible, 100).expect("second plan");
    assert_eq!(second_plan.entries[0].id, first.id);
    let second = compactor(&harness, provider.clone(), None)
        .compact(&conversation, &second_plan)
        .expect("second compact")
        .expect("second summary");
    assert_eq!(summary_text(&second), Some("Second summary."));
    let request = &provider.requests()[1];
    let user = request.messages[1].blocks[0]
        .as_text()
        .expect("second summary user text");
    assert!(user.contains("First summary."), "{user}");
    let visible = harness
        .session()
        .history_for_prompt(&conversation)
        .expect("visible");
    assert_eq!(visible[0].id, second.id);
    assert!(!visible.iter().any(|entry| entry.id == first.id));
}

#[test]
fn the_memory_flush_runs_before_the_summary() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    let user = save_entry(
        &harness,
        &conversation,
        "user",
        json!({"text": "I prefer metric units."}),
    );
    save_entry(
        &harness,
        &conversation,
        "assistant",
        json!({"text": "Noted."}),
    );
    for index in 0..4 {
        let kind = if index % 2 == 0 { "user" } else { "assistant" };
        save_entry(
            &harness,
            &conversation,
            kind,
            json!({"text": "z".repeat(500)}),
        );
    }
    let history = harness.session().history(&conversation).expect("history");
    let plan = plan_compaction(&history, 100).expect("plan");
    let extraction = json!({
        "operations": [{
            "op": "add",
            "kind": "preference",
            "content": "Sam prefers metric units.",
            "tags": ["units", "preference"],
            "importance": 85,
            "evidence_entry_id": user,
            "evidence_quote": "I prefer metric units.",
        }]
    })
    .to_string();
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script(&extraction),
        summary_script("Summary after flush."),
    ]));
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let summary = compactor(&harness, provider.clone(), Some(Arc::clone(&memory)))
        .compact(&conversation, &plan)
        .expect("compact")
        .expect("summary");
    assert_eq!(summary_text(&summary), Some("Summary after flush."));
    let found = memory.search("metric", 5).expect("memory search");
    assert_eq!(found.len(), 1, "the flush did not store the preference");
    assert_eq!(found[0].source_ref.as_deref(), Some(user.as_str()));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.blocks.iter().any(|block| match block {
                hearth_model::ContentBlock::Text(text) => text.contains("durable memory"),
                _ => false,
            })),
        "the first request was not the extraction"
    );
}

#[test]
fn a_flush_failure_does_not_block_the_summary() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    save_entry(
        &harness,
        &conversation,
        "user",
        json!({"text": "I prefer metric units."}),
    );
    for index in 0..5 {
        let kind = if index % 2 == 0 { "user" } else { "assistant" };
        save_entry(
            &harness,
            &conversation,
            kind,
            json!({"text": "z".repeat(500)}),
        );
    }
    let history = harness.session().history(&conversation).expect("history");
    let plan = plan_compaction(&history, 100).expect("plan");
    let provider = Arc::new(ScriptedProvider::new(vec![
        Err(ProviderError::Transport("offline".to_owned())),
        summary_script("Summary after failed flush."),
    ]));
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let summary = compactor(&harness, provider.clone(), Some(Arc::clone(&memory)))
        .compact(&conversation, &plan)
        .expect("compact")
        .expect("summary");
    assert_eq!(summary_text(&summary), Some("Summary after failed flush."));
    assert!(
        memory.search("metric", 5).expect("search").is_empty(),
        "a failed flush stored memory"
    );
}

#[test]
fn hard_compaction_runs_before_the_answer() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    long_entries(&harness, &conversation, 6);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![
        summary_script("Hard summary."),
        text_script("Final answer."),
    ]));
    let budget = ContextBudget::new(6000, 2048, 0.65, 0.85, 300);
    let outcome = agent(&home, &harness, provider.clone(), budget)
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Final answer."));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.blocks.iter().any(|block| match block {
                hearth_model::ContentBlock::Text(text) => text.contains("Conversation range"),
                _ => false,
            })),
        "the first request was not the summary"
    );
    let visible = harness
        .session()
        .history_for_prompt(&conversation)
        .expect("visible");
    assert_eq!(visible[0].kind, "summary");
}

#[test]
fn soft_compaction_runs_after_a_settled_turn() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    long_entries(&harness, &conversation, 14);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Answer."),
        summary_script("Soft summary."),
    ]));
    let budget = ContextBudget::new(8000, 2048, 0.65, 0.85, 300);
    let agent = agent(&home, &harness, provider.clone(), budget);
    agent.run_submission(&submission.id).expect("run");
    assert!(agent.compact_soft(&conversation).expect("soft"));
    let visible = harness
        .session()
        .history_for_prompt(&conversation)
        .expect("visible");
    assert_eq!(visible[0].kind, "summary");
    assert_eq!(summary_text(&visible[0]), Some("Soft summary."));
}

#[test]
fn a_context_overflow_compacts_and_retries_once() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    long_entries(&harness, &conversation, 6);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![
        Err(ProviderError::Request {
            status: 400,
            message: "maximum context length exceeded".to_owned(),
            retryable: false,
        }),
        summary_script("Overflow summary."),
        text_script("Recovered answer."),
    ]));
    let budget = ContextBudget::new(128000, 8192, 0.65, 0.85, 100);
    let outcome = agent(&home, &harness, provider.clone(), budget)
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Recovered answer."));
    assert_eq!(provider.requests().len(), 3);
    let visible = harness
        .session()
        .history_for_prompt(&conversation)
        .expect("visible");
    assert_eq!(visible[0].kind, "summary");
}

#[test]
fn an_unpriced_model_still_records_compaction_usage() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    long_entries(&harness, &conversation, 4);
    let history = harness.session().history(&conversation).expect("history");
    let plan = plan_compaction(&history, 100).expect("plan");
    let provider = Arc::new(ScriptedProvider::new(vec![summary_script("Summary.")]));
    let mut pricing = Pricing::default();
    pricing.insert(
        "test-model",
        Price {
            input: 1_000_000,
            cache_read: 0,
            cache_write: 0,
            output: 2_000_000,
            reasoning: 0,
        },
    );
    let budget = ContextBudget::default();
    let compactor = Compactor::new(
        harness.session().clone(),
        provider,
        pricing,
        None,
        "test-model".to_owned(),
        "scripted".to_owned(),
        "test-owner".to_owned(),
        budget.reserve,
        budget.max_output_tokens,
    );
    compactor.compact(&conversation, &plan).expect("compact");
    let usage = harness
        .session()
        .latest_usage()
        .expect("usage")
        .expect("usage row");
    assert_eq!(usage.kind, "compaction");
    assert!(usage.cost_micros > 0);
}
