use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use hearth_core::{Harness, NewTask, Save, Submission, SubmissionStatus, Task, TaskStatus};
use hearth_memory::{MemoryKind, MemoryService, MemorySource, MemoryStatus, MemoryTool, SourceKind};
use hearth_model::{
    ChatRequest, ContentBlock, Price, Pricing, Provider, ProviderError, ProviderStream, Role,
    StreamEvent, ToolCall, Usage,
};
use hearth_runtime::{Agent, AgentConfig, AgentError, EventSink, RunEvent};
use hearth_tools::{FileTool, Tool, ToolRegistry};
use serde_json::json;
use std::fs;
use std::path::Path;
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

fn agent_with(
    home: &Path,
    harness: &Harness,
    provider: Arc<dyn Provider>,
    pricing: Pricing,
    max_turns: u32,
    tools: ToolRegistry,
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
            max_turns,
            tools,
            spill_dir: home.join("spill"),
        },
    )
}

fn agent<P: Provider + 'static>(
    home: &Path,
    harness: &Harness,
    provider: Arc<P>,
    pricing: Pricing,
) -> Agent {
    agent_with(home, harness, provider, pricing, 4, ToolRegistry::default())
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
fn an_unknown_tool_call_is_reported_and_the_run_continues() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(vec![
            Ok(StreamEvent::ToolCall(ToolCall {
                id: "call-1".to_owned(),
                name: "nope".to_owned(),
                arguments: json!({}),
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("tool_calls".to_owned()),
            }),
        ]),
        Ok(vec![
            Ok(StreamEvent::TextDelta("recovered".to_owned())),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ]),
    ]));

    let outcome = agent(home.path(), &harness, provider, Pricing::default())
        .run_submission(&submission.id)
        .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    assert_eq!(outcome.answer_text.as_deref(), Some("recovered"));
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 4);
    assert_eq!(history[1].kind, "assistant");
    assert_eq!(history[2].kind, "tool_result");
    let content = history[2].content[0]["content"]
        .as_str()
        .expect("tool result content");
    assert!(content.contains("unknown tool"), "{content}");
    assert_eq!(history[3].content[0]["text"], "recovered");
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

#[test]
fn a_files_tool_call_is_executed_and_paired() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let workspace = home.path().join("workspace");
    fs::create_dir_all(workspace.join("notes")).expect("create notes");
    fs::write(workspace.join("notes/a.txt"), "alpha").expect("write file");
    let tools = ToolRegistry::new(vec![Arc::new(FileTool::new(
        workspace,
        home.path().join("spill"),
    ))]);
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(vec![
            Ok(StreamEvent::ToolCall(ToolCall {
                id: "call-1".to_owned(),
                name: "files".to_owned(),
                arguments: json!({"action": "read", "path": "notes/a.txt"}),
            })),
            Ok(StreamEvent::Usage(Usage {
                input_tokens: 8,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 2,
                reasoning_tokens: 0,
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("tool_calls".to_owned()),
            }),
        ]),
        Ok(vec![
            Ok(StreamEvent::TextDelta("read it".to_owned())),
            Ok(StreamEvent::Usage(Usage {
                input_tokens: 12,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 3,
                reasoning_tokens: 0,
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ]),
    ]));

    let outcome = agent_with(
        home.path(),
        &harness,
        provider.clone(),
        Pricing::default(),
        4,
        tools,
    )
    .run_submission(&submission.id)
    .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    assert_eq!(outcome.answer_text.as_deref(), Some("read it"));
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 4, "history: {history:#?}");
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[1].kind, "assistant");
    assert_eq!(history[2].kind, "tool_result");
    assert_eq!(history[2].content[0]["call_id"], "call-1");
    assert_eq!(history[2].content[0]["content"], "alpha");
    assert_eq!(history[3].kind, "assistant");
    assert_eq!(history[3].content[0]["text"], "read it");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().any(|message| {
        message.role == Role::Tool
            && message.blocks.iter().any(|block| {
                matches!(block, ContentBlock::ToolResult { call_id, .. } if call_id == "call-1")
            })
    }));

    let usage = harness
        .session()
        .usage_for_task(&outcome.task_id)
        .expect("usage");
    assert_eq!(usage.len(), 2);
    assert_eq!(usage[0].input_tokens, 8);
    assert_eq!(usage[1].input_tokens, 12);
}

#[test]
fn a_large_file_read_spills_and_can_be_read_back() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let workspace = home.path().join("workspace");
    fs::create_dir_all(workspace.join("notes")).expect("create notes");
    let full = format!("{}{}", "a".repeat(20_000), "TAIL");
    fs::write(workspace.join("notes/large.txt"), &full).expect("write file");
    let tools = ToolRegistry::new(vec![Arc::new(FileTool::new(
        workspace.clone(),
        home.path().join("spill"),
    ))]);
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(vec![
            Ok(StreamEvent::ToolCall(ToolCall {
                id: "call-1".to_owned(),
                name: "files".to_owned(),
                arguments: json!({"action": "read", "path": "notes/large.txt"}),
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("tool_calls".to_owned()),
            }),
        ]),
        Ok(vec![
            Ok(StreamEvent::TextDelta("done".to_owned())),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ]),
    ]));

    let outcome = agent_with(
        home.path(),
        &harness,
        provider,
        Pricing::default(),
        4,
        tools,
    )
    .run_submission(&submission.id)
    .expect("run submission");
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);

    let history = harness.session().history(&conversation).expect("history");
    let summary = history[2].content[0]["content"]
        .as_str()
        .expect("tool result content");
    assert!(summary.contains("tool output truncated"), "{summary}");
    assert!(summary.contains("spill/call-1.txt"), "{summary}");

    let spill_path = home.path().join("spill/call-1.txt");
    assert!(spill_path.exists(), "spill file missing");
    assert_eq!(fs::read_to_string(&spill_path).expect("read spill"), full);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&spill_path)
            .expect("spill metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "spill mode was {mode:o}");
    }

    let reader = FileTool::new(workspace, home.path().join("spill"));
    let read_back = reader
        .call(&json!({"action": "read", "path": "spill/call-1.txt"}))
        .expect("read spill through the tool");
    assert_eq!(read_back.content, full);
}

#[test]
fn the_turn_limit_stops_a_tool_loop() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let workspace = home.path().join("workspace");
    fs::create_dir_all(workspace.join("notes")).expect("create notes");
    fs::write(workspace.join("notes/a.txt"), "alpha").expect("write file");
    let tools = ToolRegistry::new(vec![Arc::new(FileTool::new(
        workspace,
        home.path().join("spill"),
    ))]);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![
        Ok(StreamEvent::ToolCall(ToolCall {
            id: "call-1".to_owned(),
            name: "files".to_owned(),
            arguments: json!({"action": "read", "path": "notes/a.txt"}),
        })),
        Ok(StreamEvent::Done {
            finish_reason: Some("tool_calls".to_owned()),
        }),
    ])]));

    let outcome = agent_with(
        home.path(),
        &harness,
        provider.clone(),
        Pricing::default(),
        1,
        tools,
    )
    .run_submission(&submission.id)
    .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Unanswered);
    assert_eq!(
        outcome.submission.error,
        Some(json!({"reason": "turn_limit"}))
    );
    assert!(
        outcome
            .notice
            .as_deref()
            .expect("notice")
            .to_lowercase()
            .contains("turn limit")
    );
    assert_eq!(provider.requests().len(), 1);
    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Failed);
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 4);
    assert_eq!(history[1].kind, "assistant");
    assert_eq!(history[2].kind, "tool_result");
    assert_eq!(history[3].kind, "notice");
}

#[test]
fn a_pending_tool_call_is_replayed_before_the_next_request() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let workspace = home.path().join("workspace");
    fs::create_dir_all(workspace.join("notes")).expect("create notes");
    fs::write(workspace.join("notes/a.txt"), "alpha").expect("write file");
    harness
        .session()
        .save(Save::new(&conversation).entry(
            "assistant",
            json!([{
                "type": "tool_call",
                "id": "call-1",
                "name": "files",
                "arguments": {"action": "read", "path": "notes/a.txt"}
            }]),
        ))
        .expect("seed assistant call");
    let tools = ToolRegistry::new(vec![Arc::new(FileTool::new(
        workspace,
        home.path().join("spill"),
    ))]);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![
        Ok(StreamEvent::TextDelta("recovered".to_owned())),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])]));

    let outcome = agent_with(
        home.path(),
        &harness,
        provider.clone(),
        Pricing::default(),
        4,
        tools,
    )
    .run_submission(&submission.id)
    .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    assert_eq!(outcome.answer_text.as_deref(), Some("recovered"));
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 4);
    assert_eq!(history[1].kind, "assistant");
    assert_eq!(history[2].kind, "tool_result");
    assert_eq!(history[2].content[0]["call_id"], "call-1");
    assert_eq!(history[2].content[0]["content"], "alpha");
    assert_eq!(history[3].content[0]["text"], "recovered");
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].messages.iter().any(|message| {
        message.role == Role::Tool
            && message.blocks.iter().any(|block| {
                matches!(block, ContentBlock::ToolResult { call_id, .. } if call_id == "call-1")
            })
    }));
}

#[test]
fn a_cancelled_run_before_the_stream_settles_cancelled() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![Ok(
        StreamEvent::TextDelta("never".to_owned()),
    )])]));
    let agent = agent(home.path(), &harness, provider.clone(), Pricing::default());
    let cancel = Arc::new(AtomicBool::new(true));

    let outcome = agent
        .run_submission_with_cancel(&submission.id, cancel)
        .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Cancelled);
    assert_eq!(outcome.notice.as_deref(), Some("Stopped."));
    assert!(outcome.answer_entry_id.is_none());
    assert!(provider.requests().is_empty(), "the provider was called");
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 2, "history was {history:?}");
    assert_eq!(history[1].kind, "notice");
    assert_eq!(history[1].content["text"], "Stopped.");
    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Aborted);
    assert!(
        harness.session().latest_usage().expect("usage").is_none(),
        "no provider call should mean no usage row"
    );
}

struct CancelDuringProvider {
    cancel: Arc<AtomicBool>,
    requests: Mutex<usize>,
}

impl Provider for CancelDuringProvider {
    fn stream(&self, _request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        *self.requests.lock().expect("request lock") += 1;
        let events = vec![
            Ok(StreamEvent::TextDelta("partial".to_owned())),
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
        ];
        Ok(Box::new(CancelAfterFirst {
            events: events.into_iter(),
            cancel: Arc::clone(&self.cancel),
            first: true,
        }))
    }
}

struct CancelAfterFirst {
    events: std::vec::IntoIter<Result<StreamEvent, ProviderError>>,
    cancel: Arc<AtomicBool>,
    first: bool,
}

impl Iterator for CancelAfterFirst {
    type Item = Result<StreamEvent, ProviderError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.first {
            self.first = false;
            self.cancel.store(true, Ordering::Relaxed);
        }
        self.events.next()
    }
}

#[test]
fn a_run_cancelled_during_the_stream_discards_the_answer_and_records_usage() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let cancel = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(CancelDuringProvider {
        cancel: Arc::clone(&cancel),
        requests: Mutex::new(0),
    });
    let agent = agent(home.path(), &harness, provider.clone(), Pricing::default());

    let outcome = agent
        .run_submission_with_cancel(&submission.id, cancel)
        .expect("run submission");

    assert_eq!(outcome.submission.status, SubmissionStatus::Cancelled);
    assert_eq!(outcome.notice.as_deref(), Some("Stopped."));
    assert!(outcome.answer_entry_id.is_none());
    assert_eq!(*provider.requests.lock().expect("request lock"), 1);
    let history = harness.session().history(&conversation).expect("history");
    assert_eq!(history.len(), 2, "history was {history:?}");
    assert_eq!(history[1].kind, "notice");
    assert_eq!(history[1].content["text"], "Stopped.");
    assert!(
        !history.iter().any(|entry| entry.kind == "assistant"),
        "an assistant entry was saved: {history:?}"
    );
    let usage = harness
        .session()
        .latest_usage()
        .expect("usage")
        .expect("usage row");
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 5);
    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("read task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Aborted);
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<RunEvent>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, event: RunEvent) {
        self.events.lock().expect("event lock").push(event);
    }
}

fn event_kinds(sink: &RecordingSink) -> Vec<String> {
    sink.events
        .lock()
        .expect("event lock")
        .iter()
        .map(|event| event.kind.to_owned())
        .collect()
}

#[test]
fn a_text_run_emits_live_events_in_order() {
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
    let sink = Arc::new(RecordingSink::default());
    let outcome = agent(home.path(), &harness, provider, Pricing::default())
        .run_submission_with_events(
            &submission.id,
            Arc::new(AtomicBool::new(false)),
            sink.clone(),
        )
        .expect("run");
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    let kinds = event_kinds(&sink);
    let index = |kind: &str| kinds.iter().position(|value| value == kind);
    assert!(index("run_started").is_some(), "kinds: {kinds:?}");
    assert!(index("turn_started").is_some(), "kinds: {kinds:?}");
    assert!(index("text_delta").is_some(), "kinds: {kinds:?}");
    assert!(index("usage").is_some(), "kinds: {kinds:?}");
    assert!(index("run_settled").is_some(), "kinds: {kinds:?}");
    assert!(index("run_started") < index("turn_started"));
    assert!(index("turn_started") < index("text_delta"));
    assert!(index("text_delta") < index("usage"));
    assert!(index("usage") < index("run_settled"));
    let events = sink.events.lock().expect("event lock");
    let settled = events
        .iter()
        .find(|event| event.kind == "run_settled")
        .expect("settled");
    assert_eq!(settled.data["status"], "done");
}

#[test]
fn a_cancelled_run_emits_a_settled_event() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(Vec::new()));
    let sink = Arc::new(RecordingSink::default());
    let outcome = agent(home.path(), &harness, provider, Pricing::default())
        .run_submission_with_events(
            &submission.id,
            Arc::new(AtomicBool::new(true)),
            sink.clone(),
        )
        .expect("run");
    assert_eq!(outcome.submission.status, SubmissionStatus::Cancelled);
    let events = sink.events.lock().expect("event lock");
    let settled = events
        .iter()
        .find(|event| event.kind == "run_settled")
        .expect("settled event");
    assert_eq!(settled.data["status"], "cancelled");
}

#[test]
fn a_tool_run_emits_start_and_result_events() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let workspace = home.path().join("workspace");
    fs::create_dir_all(workspace.join("notes")).expect("create notes");
    fs::write(workspace.join("notes/a.txt"), "alpha").expect("write file");
    let tools = ToolRegistry::new(vec![Arc::new(FileTool::new(
        workspace,
        home.path().join("spill"),
    ))]);
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(vec![
            Ok(StreamEvent::ToolCall(ToolCall {
                id: "call-1".to_owned(),
                name: "files".to_owned(),
                arguments: json!({"action": "read", "path": "notes/a.txt"}),
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("tool_calls".to_owned()),
            }),
        ]),
        Ok(vec![
            Ok(StreamEvent::TextDelta("read it".to_owned())),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ]),
    ]));
    let sink = Arc::new(RecordingSink::default());
    let outcome = agent_with(
        home.path(),
        &harness,
        provider,
        Pricing::default(),
        4,
        tools,
    )
    .run_submission_with_events(
        &submission.id,
        Arc::new(AtomicBool::new(false)),
        sink.clone(),
    )
    .expect("run");
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    let kinds = event_kinds(&sink);
    assert!(
        kinds.iter().any(|kind| kind == "tool_call_started"),
        "kinds: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "tool_result"),
        "kinds: {kinds:?}"
    );
}

fn message_text(message: &hearth_model::ChatMessage) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn text_script(answer: &str) -> Script {
    Ok(vec![
        Ok(StreamEvent::TextDelta(answer.to_owned())),
        Ok(StreamEvent::Usage(Usage {
            input_tokens: 10,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 3,
            reasoning_tokens: 0,
        })),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])
}

fn json_script(text: &str) -> Script {
    Ok(vec![
        Ok(StreamEvent::TextDelta(text.to_owned())),
        Ok(StreamEvent::Usage(Usage {
            input_tokens: 10,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 3,
            reasoning_tokens: 0,
        })),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])
}

fn noop_script() -> Script {
    json_script(r#"{"operations":[{"op":"noop"}]}"#)
}

fn submit_text(
    harness: &Harness,
    conversation: &str,
    request_id: &str,
    text: &str,
) -> (Submission, String) {
    let submission = harness
        .session()
        .submit(conversation, request_id, "input", json!({"text": text}))
        .expect("submit")
        .submission;
    let user_entry = harness
        .session()
        .history(conversation)
        .expect("history")
        .into_iter()
        .rev()
        .find(|entry| entry.kind == "user")
        .expect("user entry")
        .id;
    (submission, user_entry)
}

fn latest_extract_task(harness: &Harness, conversation: &str) -> Option<Task> {
    let id: Option<String> = harness
        .session()
        .storage()
        .read(|reader| {
            reader.query_optional(
                "SELECT id FROM tasks WHERE kind = 'memory.extract' AND conversation_id = ? \
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                (conversation,),
                |row| row.get(0),
            )
        })
        .expect("task query");
    id.and_then(|id| harness.session().task(&id).expect("task"))
}

fn memory_input(
    content: &str,
    kind: hearth_memory::MemoryKind,
    importance: i64,
) -> hearth_memory::NewMemoryInput {
    hearth_memory::NewMemoryInput {
        content: content.to_owned(),
        kind,
        tags: Vec::new(),
        importance,
        source_kind: hearth_memory::SourceKind::User,
        source_ref: None,
        supersedes: None,
        valid_from: None,
        valid_to: None,
    }
}

#[test]
fn the_agent_injects_standing_and_recall_memory() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(hearth_memory::MemoryService::new(harness.session().clone()));
    memory
        .save(memory_input(
            "Use metric units",
            hearth_memory::MemoryKind::Rule,
            90,
        ))
        .expect("standing rule");
    memory
        .save(memory_input(
            "Sam prefers dark roast coffee",
            hearth_memory::MemoryKind::Preference,
            70,
        ))
        .expect("recall memory");
    let submission = harness
        .session()
        .submit(
            &conversation,
            "req-memory",
            "input",
            json!({"text": "what coffee do I like?"}),
        )
        .expect("submit")
        .submission;
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Dark roast."),
        noop_script(),
    ]));
    let outcome = agent(
        home.path(),
        &harness,
        Arc::clone(&provider),
        Pricing::default(),
    )
    .with_memory(Arc::clone(&memory))
    .run_submission(&submission.id)
    .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Dark roast."));

    let requests = provider.requests();
    let request = requests.first().expect("request");
    let texts: Vec<String> = request.messages.iter().map(message_text).collect();
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Standing memory") && text.contains("Use metric units")),
        "missing standing: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Relevant memory") && text.contains("dark roast coffee")),
        "missing recall: {texts:?}"
    );
}

#[test]
fn a_trivial_message_gets_standing_but_no_recall() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(hearth_memory::MemoryService::new(harness.session().clone()));
    memory
        .save(memory_input(
            "Use metric units",
            hearth_memory::MemoryKind::Rule,
            90,
        ))
        .expect("standing rule");
    memory
        .save(memory_input(
            "Sam prefers dark roast coffee",
            hearth_memory::MemoryKind::Preference,
            70,
        ))
        .expect("recall memory");
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![text_script("Hello.")]));
    agent(
        home.path(),
        &harness,
        Arc::clone(&provider),
        Pricing::default(),
    )
    .with_memory(memory)
    .run_submission(&submission.id)
    .expect("run");

    let requests = provider.requests();
    let request = requests.last().expect("request");
    let texts: Vec<String> = request.messages.iter().map(message_text).collect();
    assert!(
        texts.iter().any(|text| text.contains("Standing memory")),
        "missing standing: {texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text.contains("Relevant memory")),
        "trivial message got recall: {texts:?}"
    );
}

#[test]
fn a_memory_failure_does_not_stop_the_answer() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(hearth_memory::MemoryService::new(harness.session().clone()));
    memory
        .save(memory_input(
            "Sam prefers dark roast coffee",
            hearth_memory::MemoryKind::Preference,
            70,
        ))
        .expect("memory");
    harness
        .session()
        .storage()
        .write(|writer| -> Result<(), hearth_storage::StorageError> {
            writer.execute("DROP TABLE memory_fts", [])?;
            Ok(())
        })
        .expect("drop fts");
    let submission = harness
        .session()
        .submit(
            &conversation,
            "req-memory-fail",
            "input",
            json!({"text": "what coffee do I like?"}),
        )
        .expect("submit")
        .submission;
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Still here."),
        noop_script(),
    ]));
    let outcome = agent(
        home.path(),
        &harness,
        Arc::clone(&provider),
        Pricing::default(),
    )
    .with_memory(memory)
    .run_submission(&submission.id)
    .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Still here."));
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
}

// ---- M4c: post-run memory extraction ----

#[test]
fn an_answer_is_extracted_into_memory() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let (submission, user_entry) = submit_text(
        &harness,
        &conversation,
        "req-extract",
        "I prefer dark roast coffee",
    );
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let extraction = format!(
        r#"{{"operations":[{{"op":"add","kind":"preference","content":"Sam prefers dark roast coffee","tags":["coffee"],"importance":70,"evidence_entry_id":"{}","evidence_quote":"I prefer dark roast coffee"}}]}}"#,
        user_entry
    );
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Noted."),
        json_script(&extraction),
    ]));

    let outcome = agent(
        home.path(),
        &harness,
        Arc::clone(&provider),
        Pricing::default(),
    )
    .with_memory(Arc::clone(&memory))
    .run_submission(&submission.id)
    .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Noted."));
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);

    let found = memory.search("dark roast", 5).expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source_kind, SourceKind::Session);
    assert_eq!(found[0].source_ref.as_deref(), Some(user_entry.as_str()));

    let task = latest_extract_task(&harness, &conversation).expect("extraction task");
    assert_eq!(task.status, TaskStatus::Done);
    let result = task.result.as_ref().expect("result");
    assert_eq!(result["applied"], 1);
    let usage = harness.session().usage_for_task(&task.id).expect("usage");
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].kind, "memory.extract");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let extraction_text: String = requests[1]
        .messages
        .iter()
        .map(message_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(extraction_text.contains("I prefer dark roast coffee"));
    assert!(extraction_text.contains("operations"));

    let prompt = harness
        .session()
        .latest_prompt()
        .expect("blob")
        .expect("prompt");
    assert!(prompt.text.contains("I prefer dark roast coffee"));
}

#[test]
fn an_extraction_without_valid_evidence_is_rejected() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let (submission, _user_entry) = submit_text(
        &harness,
        &conversation,
        "req-extract-bad",
        "I prefer dark roast coffee",
    );
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let extraction = r#"{"operations":[{"op":"add","kind":"preference","content":"Sam prefers dark roast coffee","evidence_entry_id":"missing","evidence_quote":"I prefer dark roast coffee"}]}"#;
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Noted."),
        json_script(extraction),
    ]));

    agent(home.path(), &harness, provider, Pricing::default())
        .with_memory(Arc::clone(&memory))
        .run_submission(&submission.id)
        .expect("run");

    assert_eq!(memory.stats().expect("stats").total, 0);
    let task = latest_extract_task(&harness, &conversation).expect("extraction task");
    assert_eq!(task.status, TaskStatus::Done);
    let result = task.result.as_ref().expect("result");
    assert_eq!(result["applied"], 0);
    assert_eq!(result["rejected"], 1);
}

#[test]
fn a_noop_extraction_writes_nothing() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let (submission, _user_entry) = submit_text(
        &harness,
        &conversation,
        "req-extract-noop",
        "I prefer dark roast coffee",
    );
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Noted."),
        noop_script(),
    ]));

    agent(home.path(), &harness, provider, Pricing::default())
        .with_memory(Arc::clone(&memory))
        .run_submission(&submission.id)
        .expect("run");

    assert_eq!(memory.stats().expect("stats").total, 0);
    let task = latest_extract_task(&harness, &conversation).expect("extraction task");
    assert_eq!(task.status, TaskStatus::Done);
}

#[test]
fn a_trivial_message_skips_extraction() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let provider = Arc::new(ScriptedProvider::new(vec![text_script("Hello.")]));

    agent(
        home.path(),
        &harness,
        Arc::clone(&provider),
        Pricing::default(),
    )
    .with_memory(Arc::clone(&memory))
    .run_submission(&submission.id)
    .expect("run");

    assert_eq!(provider.requests().len(), 1);
    assert!(latest_extract_task(&harness, &conversation).is_none());
    assert_eq!(memory.stats().expect("stats").total, 0);
}

#[test]
fn a_malformed_extraction_fails_the_task_but_not_the_answer() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let (submission, _user_entry) = submit_text(
        &harness,
        &conversation,
        "req-extract-malformed",
        "I prefer dark roast coffee",
    );
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Still answered."),
        json_script("not json"),
    ]));

    let outcome = agent(home.path(), &harness, provider, Pricing::default())
        .with_memory(Arc::clone(&memory))
        .run_submission(&submission.id)
        .expect("run");

    assert_eq!(outcome.answer_text.as_deref(), Some("Still answered."));
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    assert_eq!(memory.stats().expect("stats").total, 0);
    let task = latest_extract_task(&harness, &conversation).expect("extraction task");
    assert_eq!(task.status, TaskStatus::Failed);
    assert!(task.result.as_ref().expect("result")["error"].is_string());
}

#[test]
fn the_extraction_request_contains_the_run_and_similar_memories() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    memory
        .save(memory_input(
            "Sam prefers dark roast coffee",
            MemoryKind::Preference,
            60,
        ))
        .expect("seed memory");
    let (submission, _user_entry) = submit_text(
        &harness,
        &conversation,
        "req-extract-similar",
        "I prefer dark roast coffee",
    );
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Noted."),
        noop_script(),
    ]));

    agent(
        home.path(),
        &harness,
        Arc::clone(&provider),
        Pricing::default(),
    )
    .with_memory(memory)
    .run_submission(&submission.id)
    .expect("run");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let extraction_text: String = requests[1]
        .messages
        .iter()
        .map(message_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(extraction_text.contains("Sam prefers dark roast coffee"));
}

#[test]
fn an_extraction_can_update_an_existing_memory() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let seed = memory
        .save(memory_input("Sam prefers tea", MemoryKind::Preference, 50))
        .expect("seed");
    let (submission, user_entry) =
        submit_text(&harness, &conversation, "req-update", "I prefer coffee now");
    let extraction = format!(
        r#"{{"operations":[{{"op":"update","kind":"preference","content":"Sam prefers coffee","tags":["coffee"],"importance":70,"evidence_entry_id":"{}","evidence_quote":"I prefer coffee now","supersedes_id":"{}"}}]}}"#,
        user_entry, seed.id
    );
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Updated."),
        json_script(&extraction),
    ]));

    let outcome = agent(home.path(), &harness, provider, Pricing::default())
        .with_memory(Arc::clone(&memory))
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Updated."));
    let updated = memory.get(&seed.id).expect("get").expect("row");
    assert_eq!(updated.content, "Sam prefers coffee");
    assert_eq!(updated.importance, 70);
    assert_eq!(memory.stats().expect("stats").total, 1);
    let task = latest_extract_task(&harness, &conversation).expect("task");
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.result.as_ref().expect("result")["applied"], 1);
}

#[test]
fn an_extraction_can_supersede_an_existing_memory() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let seed = memory
        .save(memory_input("Sam lives in Manila", MemoryKind::Fact, 50))
        .expect("seed");
    let (submission, user_entry) = submit_text(
        &harness,
        &conversation,
        "req-supersede",
        "I moved to Quezon City",
    );
    let extraction = format!(
        r#"{{"operations":[{{"op":"supersede","kind":"fact","content":"Sam lives in Quezon City","tags":[],"importance":60,"evidence_entry_id":"{}","evidence_quote":"I moved to Quezon City","supersedes_id":"{}"}}]}}"#,
        user_entry, seed.id
    );
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_script("Moved."),
        json_script(&extraction),
    ]));

    let outcome = agent(home.path(), &harness, provider, Pricing::default())
        .with_memory(Arc::clone(&memory))
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Moved."));
    let old = memory.get(&seed.id).expect("get").expect("old");
    assert_eq!(old.status, MemoryStatus::Superseded);
    let found = memory.search("Quezon City", 5).expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].content, "Sam lives in Quezon City");
    assert_eq!(memory.stats().expect("stats").total, 2);
    let task = latest_extract_task(&harness, &conversation).expect("task");
    assert_eq!(task.status, TaskStatus::Done);
}

#[test]
fn a_memory_tool_call_uses_the_run_source() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let entry = harness
        .session()
        .latest_user_entry_id(&conversation)
        .expect("latest")
        .expect("entry");
    let memory = Arc::new(MemoryService::new(harness.session().clone()));
    let tools = ToolRegistry::new(vec![Arc::new(MemoryTool::with_source(
        Arc::clone(&memory),
        MemorySource::user(entry.clone()),
    ))]);
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(vec![
            Ok(StreamEvent::ToolCall(ToolCall {
                id: "call-mem".to_owned(),
                name: "memory".to_owned(),
                arguments: json!({
                    "action": "save",
                    "content": "Sam prefers metric units",
                    "kind": "preference",
                    "importance": 85
                }),
            })),
            Ok(StreamEvent::Usage(Usage {
                input_tokens: 8,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 2,
                reasoning_tokens: 0,
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("tool_calls".to_owned()),
            }),
        ]),
        Ok(vec![
            Ok(StreamEvent::TextDelta("saved".to_owned())),
            Ok(StreamEvent::Usage(Usage {
                input_tokens: 12,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 2,
                reasoning_tokens: 0,
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ]),
    ]));

    let outcome = agent_with(
        home.path(),
        &harness,
        provider,
        Pricing::default(),
        4,
        tools,
    )
    .run_submission(&submission.id)
    .expect("run submission");
    assert_eq!(outcome.answer_text.as_deref(), Some("saved"));

    let items = memory.search("metric", 10).expect("search");
    let item = items
        .iter()
        .find(|item| item.content.contains("metric units"))
        .expect("saved memory");
    assert_eq!(item.source_kind, SourceKind::User);
    assert_eq!(item.source_ref.as_deref(), Some(entry.as_str()));
    let action: String = harness
        .session()
        .storage()
        .read(|reader| {
            reader.query_row(
                "SELECT action FROM memory_events ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
        })
        .expect("event");
    assert_eq!(action, "add");
}
