use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use hearth_core::{
    ContextBudget, Harness, NewEntry, NewTask, Save, Submission, SubmissionStatus, TaskStatus,
};
use hearth_memory::{MemoryInput, MemorySearchOptions, MemoryService, SearchMode};
use hearth_model::{
    ChatRequest, ContentBlock, Provider, ProviderError, ProviderStream, Role, StreamEvent,
    ToolCall, Usage,
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
    tools: ToolRegistry,
) -> Agent {
    Agent::new(
        harness.session().clone(),
        provider,
        AgentConfig {
            soul: "You are a test soul.".to_owned(),
            model: "test-model".to_owned(),
            provider_name: "scripted".to_owned(),
            owner: "test-owner".to_owned(),
            tools,
            spill_dir: home.join("spill"),
            guidance: String::new(),
        },
    )
}

fn agent<P: Provider + 'static>(home: &Path, harness: &Harness, provider: Arc<P>) -> Agent {
    agent_with(home, harness, provider, ToolRegistry::default())
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
    let outcome = agent(home.path(), &harness, Arc::clone(&provider))
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
    assert_eq!(requests[0].max_tokens, None);
    assert_eq!(requests[0].temperature, None);
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

    let outcome = agent(home.path(), &harness, provider)
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
    let error = agent(home.path(), &harness, provider)
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

    let outcome = agent(home.path(), &harness, provider)
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

    let outcome = agent(home.path(), &harness, provider)
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
    let agent = Arc::new(agent(home.path(), &harness, provider));

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
    assert_eq!(
        after.status,
        SubmissionStatus::Done,
        "the active run absorbs a queued submission and settles it"
    );
    assert_eq!(after.answer_entry_id, first_outcome.answer_entry_id);
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

    let outcome = agent_with(home.path(), &harness, provider.clone(), tools)
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

    let outcome = agent_with(home.path(), &harness, provider, tools)
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
fn a_long_tool_loop_is_not_cut_short_by_a_turn_limit() {
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
    // Well past the 120 requests the retired `agent.max_turns` setting allowed.
    let rounds = 125;
    let mut scripts = Vec::with_capacity(rounds + 1);
    for round in 0..rounds {
        scripts.push(Ok(vec![
            Ok(StreamEvent::ToolCall(ToolCall {
                id: format!("call-{round}"),
                name: "files".to_owned(),
                arguments: json!({"action": "read", "path": "notes/a.txt"}),
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("tool_calls".to_owned()),
            }),
        ]));
    }
    scripts.push(Ok(vec![
        Ok(StreamEvent::TextDelta("done".to_owned())),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ]));
    let provider = Arc::new(ScriptedProvider::new(scripts));

    let outcome = agent_with(home.path(), &harness, provider.clone(), tools)
        .run_submission(&submission.id)
        .expect("run submission");

    assert_eq!(outcome.answer_text.as_deref(), Some("done"));
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
    assert_eq!(provider.requests().len(), rounds + 1);
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

    let outcome = agent_with(home.path(), &harness, provider.clone(), tools)
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
    let agent = agent(home.path(), &harness, provider.clone());
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
    let agent = agent(home.path(), &harness, provider.clone());

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
    let outcome = agent(home.path(), &harness, provider)
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
    assert!(index("assistant_reply").is_some(), "kinds: {kinds:?}");
    assert!(index("user_message").is_some(), "kinds: {kinds:?}");
    assert!(
        index("text_delta").is_none(),
        "deltas must not stream: {kinds:?}"
    );
    assert!(index("usage").is_some(), "kinds: {kinds:?}");
    assert!(index("run_settled").is_some(), "kinds: {kinds:?}");
    assert!(index("run_started") < index("turn_started"));
    assert!(index("turn_started") < index("assistant_reply"));
    assert!(index("assistant_reply") < index("usage"));
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
    let outcome = agent(home.path(), &harness, provider)
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
    let outcome = agent_with(home.path(), &harness, provider, tools)
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

/// Seed one long-term memory and return its id.
fn seed_memory(memory: &MemoryService, content: &str, tags: &[&str]) -> String {
    memory
        .store()
        .add(&MemoryInput {
            content: content.to_owned(),
            tags: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
            source: None,
        })
        .expect("memory")
        .0
        .id
}

#[test]
fn the_agent_injects_standing_and_recall_memory() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(MemoryService::new(harness.session().clone()).expect("memory"));
    // A bullet in the standing file is injected without a retrieval cue.
    memory
        .standing_store()
        .add("Use metric units", 90)
        .expect("standing rule");
    // A long-term fact is retrieved by search.
    seed_memory(&memory, "Sam prefers dark roast coffee", &["coffee"]);
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
    let provider = Arc::new(ScriptedProvider::new(vec![text_script("Dark roast.")]));
    let outcome = agent(home.path(), &harness, Arc::clone(&provider))
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
            .any(|text| text.contains("<standing_memory>") && text.contains("Use metric units")),
        "missing standing: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.contains("<relevant_memory>") && text.contains("dark roast coffee")),
        "missing recall: {texts:?}"
    );
}

#[test]
fn a_trivial_message_gets_standing_but_no_recall() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(MemoryService::new(harness.session().clone()).expect("memory"));
    memory
        .standing_store()
        .add("Use metric units", 90)
        .expect("standing rule");
    seed_memory(&memory, "Sam prefers dark roast coffee", &["coffee"]);
    // `submit` sends "hi", which carries no topic worth searching for.
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![text_script("Hello.")]));
    agent(home.path(), &harness, Arc::clone(&provider))
        .with_memory(memory)
        .run_submission(&submission.id)
        .expect("run");

    let requests = provider.requests();
    let request = requests.last().expect("request");
    let texts: Vec<String> = request.messages.iter().map(message_text).collect();
    assert!(
        texts
            .iter()
            .any(|text| text.contains("<standing_memory>") && text.contains("Use metric units")),
        "missing standing: {texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text.contains("<relevant_memory>")),
        "a trivial message got recall: {texts:?}"
    );
}

#[test]
fn a_memory_failure_does_not_stop_the_answer() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(MemoryService::new(harness.session().clone()).expect("memory"));
    seed_memory(&memory, "Sam prefers dark roast coffee", &["coffee"]);
    // Break the standing store in a way that survives the plain-file check: a
    // directory where the file belongs makes every read fail.
    let standing = memory.base().join("standing.md");
    if standing.exists() {
        fs::remove_file(&standing).expect("remove standing");
    }
    fs::create_dir(&standing).expect("block standing");
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
    let provider = Arc::new(ScriptedProvider::new(vec![text_script("Still here.")]));
    let outcome = agent(home.path(), &harness, Arc::clone(&provider))
        .with_memory(memory)
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Still here."));
    assert_eq!(outcome.submission.status, SubmissionStatus::Done);
}

#[test]
fn a_memory_tool_call_records_the_run_source() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let entry = harness
        .session()
        .latest_user_entry_id(&conversation)
        .expect("latest")
        .expect("entry");
    // The service stamps its provenance onto saves that do not name a source.
    let memory = Arc::new(
        MemoryService::new(harness.session().clone())
            .expect("memory")
            .with_source(entry.clone()),
    );
    let tools = ToolRegistry::new(memory.tools());
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(vec![
            Ok(StreamEvent::ToolCall(ToolCall {
                id: "call-mem".to_owned(),
                name: "memory_add".to_owned(),
                arguments: json!({
                    "content": "Sam prefers metric units",
                    "tags": ["units"]
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

    let outcome = agent_with(home.path(), &harness, provider, tools)
        .run_submission(&submission.id)
        .expect("run submission");
    assert_eq!(outcome.answer_text.as_deref(), Some("saved"));

    let found = memory
        .store()
        .search(
            "metric",
            SearchMode::Any,
            10,
            0,
            &MemorySearchOptions::default(),
        )
        .expect("search");
    let item = found
        .results
        .iter()
        .find(|hit| hit.memory.content.contains("metric units"))
        .expect("saved memory");
    assert_eq!(item.memory.source, entry);
}

fn tool_result_text(message: &hearth_model::ChatMessage) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn recall_is_bounded_to_fit_the_context_budget() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let memory = Arc::new(MemoryService::new(harness.session().clone()).expect("memory"));
    for index in 0..10 {
        seed_memory(
            &memory,
            &format!("project fact {index} about Atlas"),
            &["atlas"],
        );
    }
    let submission = harness
        .session()
        .submit(
            &conversation,
            "req-budget-recall",
            "input",
            json!({"text": "what do you know about project?"}),
        )
        .expect("submit")
        .submission;
    let provider = Arc::new(ScriptedProvider::new(vec![text_script("Short.")]));
    let budget = ContextBudget::new(5_320, 4_096, 0.65, 0.85, 1_000);
    let outcome = agent(home.path(), &harness, Arc::clone(&provider))
        .with_memory(Arc::clone(&memory))
        .with_budget(budget)
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Short."));

    let requests = provider.requests();
    let request = requests.first().expect("request");
    let recall = request
        .messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(message_text)
        .find(|text| text.contains("<relevant_memory>"))
        .expect("recall message");
    let lines = recall
        .lines()
        .filter(|line| line.trim_start().starts_with("- "))
        .count();
    // Ten memories match, so a bounded recall must return fewer than all of them.
    assert!(
        lines > 0 && lines < 10,
        "expected a bounded recall, got {lines} lines: {recall}"
    );
    // The ceiling is the recall cap, which is eight.
    assert!(lines <= 8, "recall exceeded the item cap, got {lines}");
}

#[test]
fn oversized_tool_results_are_shrunk_to_fit_the_context_budget() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let huge = "x".repeat(20_000);
    harness
        .session()
        .save(
            Save::new(&conversation)
                .entry_with(NewEntry {
                    kind: "assistant".to_owned(),
                    content: json!([
                        {"type": "text", "text": "checking"},
                        {"type": "tool_call", "id": "call-budget", "name": "files", "arguments": {}}
                    ]),
                    source: None,
                    prompt_hash: None,
                })
                .entry_with(NewEntry {
                    kind: "tool_result".to_owned(),
                    content: json!([{
                        "type": "tool_result",
                        "call_id": "call-budget",
                        "content": huge,
                    }]),
                    source: None,
                    prompt_hash: None,
                }),
        )
        .expect("tool result");
    let submission = harness
        .session()
        .submit(
            &conversation,
            "req-budget-tool",
            "input",
            json!({"text": "hello"}),
        )
        .expect("submit")
        .submission;
    let provider = Arc::new(ScriptedProvider::new(vec![text_script("Done.")]));
    let budget = ContextBudget::new(7_120, 4_096, 0.65, 0.85, 1_000);
    let outcome = agent(home.path(), &harness, Arc::clone(&provider))
        .with_budget(budget)
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("Done."));

    let requests = provider.requests();
    let request = requests.first().expect("request");
    let tool_texts: Vec<String> = request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .map(tool_result_text)
        .collect();
    assert!(
        tool_texts
            .iter()
            .any(|text| text.len() < 20_000 && text.contains("truncated")),
        "tool result was not shrunk: {tool_texts:?}"
    );
}

#[test]
fn an_oversized_prompt_settles_with_a_budget_notice() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let huge = "y".repeat(50_000);
    let submission = harness
        .session()
        .submit(
            &conversation,
            "req-budget-error",
            "input",
            json!({"text": huge}),
        )
        .expect("submit")
        .submission;
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let budget = ContextBudget::new(5_420, 4_096, 0.65, 0.85, 1_000);
    let outcome = agent(home.path(), &harness, Arc::clone(&provider))
        .with_budget(budget)
        .run_submission(&submission.id)
        .expect("run");
    assert_eq!(outcome.submission.status, SubmissionStatus::Unanswered);
    assert!(
        outcome
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("too large")),
        "missing budget notice: {:?}",
        outcome.notice
    );
    assert!(
        provider.requests().is_empty(),
        "the provider must not be called for an over-budget prompt"
    );
    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("task lookup")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Failed);
    assert!(
        task.result
            .as_ref()
            .is_some_and(|result| result["reason"] == "budget"),
        "task result: {:?}",
        task.result
    );
}

#[test]
fn the_voice_tag_guidance_is_not_repeated_in_the_runtime_note() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![
        Ok(StreamEvent::TextDelta("hello".to_owned())),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])]));
    let agent = Agent::new(
        harness.session().clone(),
        provider.clone(),
        AgentConfig {
            soul: "You are a test soul.".to_owned(),
            model: "test-model".to_owned(),
            provider_name: "scripted".to_owned(),
            owner: "test-owner".to_owned(),
            tools: ToolRegistry::default(),
            spill_dir: home.path().join("spill"),
            guidance: String::new(),
        },
    );
    let outcome = agent.run_submission(&submission.id).expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("hello"));
    let requests = provider.requests();
    let runtime = requests[0].messages.last().expect("runtime note");
    let text = runtime
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    // The audio-tag guidance belongs to the message tool description. Repeating
    // it in the uncached runtime note cost fresh tokens on every single turn.
    assert!(!text.contains("Eleven"), "runtime note was {text:?}");
    assert!(!text.contains("[warm"), "runtime note was {text:?}");
}

#[test]
fn project_guidance_reaches_the_prompt() {
    let (home, harness) = harness();
    let conversation = conversation(&harness);
    let submission = submit(&harness, &conversation);
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(vec![
        Ok(StreamEvent::TextDelta("hello".to_owned())),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ])]));
    let agent = Agent::new(
        harness.session().clone(),
        provider.clone(),
        AgentConfig {
            soul: "You are a test soul.".to_owned(),
            model: "test-model".to_owned(),
            provider_name: "scripted".to_owned(),
            owner: "test-owner".to_owned(),
            tools: ToolRegistry::default(),
            spill_dir: home.path().join("spill"),
            guidance: "PROJECT RULES".to_owned(),
        },
    );
    agent.run_submission(&submission.id).expect("run");
    let requests = provider.requests();
    let found = requests[0].messages.iter().any(|message| {
        message.blocks.iter().any(
            |block| matches!(block, ContentBlock::Text(text) if text.contains("PROJECT RULES")),
        )
    });
    assert!(found, "the project guidance did not reach the prompt");
}
