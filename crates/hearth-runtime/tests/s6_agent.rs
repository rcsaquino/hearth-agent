use std::collections::VecDeque;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use hearth_core::{Harness, NewTask, Submission, SubmissionStatus, TaskStatus};
use hearth_model::{
    ChatRequest, Price, Pricing, Provider, ProviderError, ProviderStream, StreamEvent, Usage,
};
use hearth_runtime::{Agent, AgentConfig, AgentError};
use hearth_tools::ToolRegistry;
use std::path::Path;
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
    submit_with(harness, conversation, "req-1")
}

fn submit_with(harness: &Harness, conversation: &str, request_id: &str) -> Submission {
    harness
        .session()
        .submit(conversation, request_id, "input", json!({"text": "hi"}))
        .expect("submit")
        .submission
}

fn agent<P: Provider + 'static>(
    home: &Path,
    harness: &Harness,
    provider: Arc<P>,
    pricing: Pricing,
) -> Agent {
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
            tools: ToolRegistry::default(),
            spill_dir: home.join("spill"),
        },
    )
}

#[test]
fn a_plain_answer_settles_the_submission_and_records_usage() {
    let (home, harness) = harness();
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

    let outcome = agent(home.path(), &harness, Arc::clone(&provider), pricing)
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
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Err(ProviderError::Request {
        status: 500,
        message: "boom".to_owned(),
        retryable: false,
    })]));

    let outcome = agent(home.path(), &harness, provider, Pricing::default())
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
    let (home, harness) = harness();
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
    let error = agent(home.path(), &harness, provider, Pricing::default())
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
    let (home, harness) = harness();
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

    let outcome = agent(home.path(), &harness, provider, Pricing::default())
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

#[test]
fn a_thinking_only_response_settles_unanswered() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![
        Ok(StreamEvent::ThinkingDelta("private reasoning".to_owned())),
        Ok(StreamEvent::Usage(Usage {
            input_tokens: 4,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 2,
            reasoning_tokens: 0,
        })),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])]));

    let outcome = agent(home.path(), &harness, provider, Pricing::default())
        .run_submission(&submission.id)
        .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Unanswered);
    assert!(outcome.answer_text.is_none());
    assert!(outcome.notice.expect("notice").contains("no answer"));
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].kind, "notice");
    assert!(
        history.iter().all(|entry| entry.kind != "assistant"),
        "a thinking-only response was saved as an answer"
    );
    let usage = harness
        .session()
        .usage_for_task(&outcome.task_id)
        .expect("usage");
    assert_eq!(usage.len(), 1);
    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Failed);
}

struct BlockingProvider {
    started: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Provider for BlockingProvider {
    fn stream(&self, _request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.started
            .lock()
            .expect("started lock")
            .send(())
            .expect("signal start");
        self.release
            .lock()
            .expect("release lock")
            .recv_timeout(Duration::from_secs(2))
            .expect("release first run");
        Ok(Box::new(
            vec![
                Ok(StreamEvent::TextDelta("done".to_owned())),
                Ok(StreamEvent::Done {
                    finish_reason: Some("stop".to_owned()),
                }),
            ]
            .into_iter(),
        ))
    }
}

#[test]
fn two_concurrent_runs_cannot_enter_one_conversation() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let first = submit_with(&harness, &conversation, "req-1");
    let second = submit_with(&harness, &conversation, "req-2");
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let provider = Arc::new(BlockingProvider {
        started: Mutex::new(started_tx),
        release: Mutex::new(release_rx),
    });
    let agent = Arc::new(agent(home.path(), &harness, provider, Pricing::default()));

    let first_id = first.id.clone();
    let runner = Arc::clone(&agent);
    let handle = thread::spawn(move || runner.run_submission(&first_id));

    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first run did not start");
    let error = agent
        .run_submission(&second.id)
        .expect_err("a second run in the same conversation must be busy");
    assert!(matches!(error, AgentError::Busy));

    release_tx.send(()).expect("release first run");
    let first_outcome = handle
        .join()
        .expect("join first run")
        .expect("first run result");
    assert_eq!(first_outcome.submission.status, SubmissionStatus::Done);

    let after = harness
        .session()
        .submission_by_id(&second.id)
        .expect("read second submission")
        .expect("second submission exists");
    assert_eq!(after.status, SubmissionStatus::Queued);
}
