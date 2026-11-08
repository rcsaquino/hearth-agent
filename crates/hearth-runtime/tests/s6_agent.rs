use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use hearth_core::{Harness, NewTask, Submission, SubmissionStatus, TaskStatus};
use hearth_model::{
    ChatRequest, Price, Pricing, Provider, ProviderError, ProviderStream, StreamEvent, Usage,
};
use hearth_runtime::{Agent, AgentConfig, AgentError};
use serde_json::json;
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
        self.requests.lock().expect("request lock").clone()
    }
}

impl Provider for ScriptedProvider {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().expect("request lock").push(request);
        let script = self
            .scripts
            .lock()
            .expect("script lock")
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
        .expect("create conversation")
        .id
}

fn submit(harness: &Harness, conversation: &str) -> Submission {
    harness
        .session()
        .submit(conversation, "req-1", "input", json!({"text": "hi"}))
        .expect("submit")
        .submission
}

fn agent(harness: &Harness, provider: Arc<ScriptedProvider>, pricing: Pricing) -> Agent {
    Agent::new(
        harness.session().clone(),
        provider,
        pricing,
        AgentConfig {
            soul: "You are a test soul.".to_owned(),
            model: "test-model".to_owned(),
            provider_name: "scripted".to_owned(),
            owner: "test-owner".to_owned(),
            max_turns: 4,
        },
    )
}

#[test]
fn a_plain_answer_settles_the_submission_and_records_usage() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![
        Ok(StreamEvent::TextDelta("hello".to_owned())),
        Ok(StreamEvent::Usage(Usage {
            input_tokens: 10,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 5,
            reasoning_tokens: 0,
        })),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])]));
    let mut pricing = Pricing::default();
    pricing.insert(
        "test-model",
        Price {
            input: 1_000_000,
            cache_read: 0,
            cache_write: 0,
            output: 4_000_000,
            reasoning: 0,
        },
    );

    let outcome = agent(&harness, Arc::clone(&provider), pricing)
        .run_submission(&submission.id)
        .expect("run submission");

    assert_eq!(outcome.answer_text.as_deref(), Some("hello"));
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    let answer_id = outcome.answer_entry_id.expect("answer entry id");
    assert_eq!(
        outcome.submission.answer_entry_id.as_deref(),
        Some(answer_id.as_str())
    );

    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[1].id, answer_id);
    assert_eq!(history[1].kind, "assistant");
    assert_eq!(history[1].content[0]["type"], "text");
    assert_eq!(history[1].content[0]["text"], "hello");
    assert!(history[1].prompt_hash.is_some());

    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.phase, "request");
    assert_eq!(task.checkpoint.as_ref().expect("checkpoint")["turn"], 0);

    let usage = harness
        .session()
        .usage_for_task(&outcome.task_id)
        .expect("usage");
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].provider, "scripted");
    assert_eq!(usage[0].model, "test-model");
    assert_eq!(usage[0].input_tokens, 10);
    assert_eq!(usage[0].output_tokens, 5);
    assert_eq!(usage[0].cost_micros, 30);
    assert!(usage[0].latency_ms >= 0);

    let prompt = harness
        .session()
        .latest_prompt()
        .expect("latest prompt")
        .expect("prompt exists");
    assert!(prompt.text.contains("test soul"));
    assert!(prompt.text.contains("hi"));

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, "test-model");
    assert_eq!(
        requests[0].session_id.as_deref(),
        Some(conversation.as_str())
    );
}

#[test]
fn a_provider_error_settles_unanswered_with_a_notice() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Err(ProviderError::Request {
        status: 500,
        message: "boom".to_owned(),
        retryable: false,
    })]));

    let outcome = agent(&harness, provider, Pricing::default())
        .run_submission(&submission.id)
        .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Unanswered);
    assert!(outcome.answer_text.is_none());
    let notice = outcome.notice.expect("notice");
    assert!(notice.contains("boom"));

    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].kind, "notice");
    assert_eq!(history[1].content["text"], notice);

    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Failed);
}

#[test]
fn a_running_generation_task_makes_the_agent_busy() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    harness
        .session()
        .create_task(
            NewTask::new(
                "generation",
                1,
                "request",
                json!({"submission_id": "other", "conversation_id": conversation}),
            )
            .conversation(&conversation),
        )
        .expect("create task");
    harness
        .session()
        .claim_task("other-worker", 60_000_000)
        .expect("claim task")
        .expect("task is claimable");

    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let error = agent(&harness, provider, Pricing::default())
        .run_submission(&submission.id)
        .expect_err("a busy conversation must be rejected");
    assert!(matches!(error, AgentError::Busy));

    let after = harness
        .session()
        .submission_by_id(&submission.id)
        .expect("read submission")
        .expect("submission exists");
    assert_eq!(after.status, SubmissionStatus::Queued);
}

#[test]
fn a_tool_call_without_tools_settles_unanswered() {
    let (_home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![
        Ok(StreamEvent::ToolCall(hearth_model::ToolCall {
            id: "call-1".to_owned(),
            name: "files".to_owned(),
            arguments: json!({"path": "a.txt"}),
        })),
        Ok(StreamEvent::Done {
            finish_reason: Some("tool_calls".to_owned()),
        }),
    ])]));

    let outcome = agent(&harness, provider, Pricing::default())
        .run_submission(&submission.id)
        .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Unanswered);
    assert!(outcome.notice.is_some());
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].kind, "notice");
    assert!(
        history.iter().all(|entry| entry.kind != "assistant"),
        "an orphaned assistant tool call was saved"
    );
}
