use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use hearth_core::{Harness, Submission, SubmissionStatus};
use hearth_model::{
    ChatRequest, ContentBlock, Provider, ProviderError, ProviderStream, StreamEvent, ToolCall,
    ToolSpec, Usage,
};
use hearth_runtime::{Agent, AgentConfig};
use hearth_tools::{Tool, ToolError, ToolOutput, ToolRegistry};
use serde_json::{Value, json};
use tempfile::TempDir;

struct Echo;

impl Tool for Echo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_owned(),
            description: "Echo the text argument.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: arguments
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            is_error: false,
        })
    }
}

struct SlowProvider {
    requests: Mutex<Vec<ChatRequest>>,
    started: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    first: Mutex<bool>,
    tool_first: bool,
}

impl SlowProvider {
    fn new(tool_first: bool) -> (Self, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        (
            Self {
                requests: Mutex::new(Vec::new()),
                started: started_tx,
                release: Mutex::new(release_rx),
                first: Mutex::new(true),
                tool_first,
            },
            started_rx,
            release_tx,
        )
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

impl Provider for SlowProvider {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().expect("requests").push(request);
        let mut first = self.first.lock().expect("first");
        if *first {
            *first = false;
            self.started.send(()).expect("started");
            self.release
                .lock()
                .expect("release")
                .recv_timeout(Duration::from_secs(5))
                .expect("release signal");
            if self.tool_first {
                return Ok(Box::new(
                    [
                        Ok(StreamEvent::ToolCall(ToolCall {
                            id: "c1".to_owned(),
                            name: "echo".to_owned(),
                            arguments: json!({"text": "tool"}),
                        })),
                        Ok(StreamEvent::Usage(Usage::default())),
                        Ok(StreamEvent::Done {
                            finish_reason: Some("tool_calls".to_owned()),
                        }),
                    ]
                    .into_iter(),
                ));
            }
            return Ok(Box::new(
                [
                    Ok(StreamEvent::TextDelta("partial".to_owned())),
                    Ok(StreamEvent::Usage(Usage::default())),
                    Ok(StreamEvent::Done {
                        finish_reason: Some("stop".to_owned()),
                    }),
                ]
                .into_iter(),
            ));
        }
        Ok(Box::new(
            [
                Ok(StreamEvent::TextDelta("final".to_owned())),
                Ok(StreamEvent::Usage(Usage::default())),
                Ok(StreamEvent::Done {
                    finish_reason: Some("stop".to_owned()),
                }),
            ]
            .into_iter(),
        ))
    }
}

fn setup() -> (TempDir, Harness, String) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("conversation")
        .id;
    (home, harness, conversation)
}

fn submit(harness: &Harness, conversation: &str, request_id: &str, text: &str) -> Submission {
    harness
        .session()
        .submit(conversation, request_id, "input", json!({"text": text}))
        .expect("submit")
        .submission
}

fn agent(home: &std::path::Path, harness: &Harness, provider: Arc<dyn Provider>) -> Agent {
    Agent::new(
        harness.session().clone(),
        provider,
        hearth_model::Pricing::default(),
        AgentConfig {
            soul: "test soul".to_owned(),
            model: "test-model".to_owned(),
            provider_name: "scripted".to_owned(),
            owner: "owner".to_owned(),
            max_turns: 6,
            tools: ToolRegistry::new(vec![Arc::new(Echo)]),
            spill_dir: home.join("spill"),
            voice_tags: false,
            guidance: String::new(),
        },
    )
}

fn message_texts(request: &ChatRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_second_message_is_folded_into_the_run_and_settled_once() {
    let (home, harness, conversation) = setup();
    let (provider, started, release) = SlowProvider::new(true);
    let provider = Arc::new(provider);
    let agent = Arc::new(agent(home.path(), &harness, provider.clone()));
    let first = submit(&harness, &conversation, "req-1", "first");
    let run = {
        let agent = Arc::clone(&agent);
        let id = first.id.clone();
        thread::spawn(move || agent.run_submission(&id))
    };
    started
        .recv_timeout(Duration::from_secs(5))
        .expect("first request started");
    let second = submit(&harness, &conversation, "req-2", "second");
    release.send(()).expect("release");
    let outcome = run.join().expect("run thread").expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("final"));

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        message_texts(&requests[1])
            .iter()
            .filter(|text| text.as_str() == "second")
            .count(),
        1,
        "the second user message must be in the next request: {:?}",
        message_texts(&requests[1])
    );

    let stored = harness
        .session()
        .submission_by_id(&second.id)
        .expect("read second")
        .expect("second exists");
    assert_eq!(stored.status, SubmissionStatus::Done);
    assert_eq!(stored.answer_entry_id, outcome.answer_entry_id);
    assert!(
        harness
            .session()
            .queued_submissions(&conversation)
            .expect("queued")
            .is_empty(),
        "an absorbed submission must not remain queued"
    );
}

#[test]
fn a_stop_settles_the_absorbed_submission_cancelled() {
    let (home, harness, conversation) = setup();
    let (provider, started, release) = SlowProvider::new(false);
    let agent = Arc::new(agent(home.path(), &harness, Arc::new(provider)));
    let first = submit(&harness, &conversation, "req-1", "first");
    let second = submit(&harness, &conversation, "req-2", "second");
    let cancel = Arc::new(AtomicBool::new(false));
    let run = {
        let agent = Arc::clone(&agent);
        let id = first.id.clone();
        let cancel = Arc::clone(&cancel);
        thread::spawn(move || agent.run_submission_with_cancel(&id, Arc::clone(&cancel)))
    };
    started
        .recv_timeout(Duration::from_secs(5))
        .expect("first request started");
    cancel.store(true, Ordering::SeqCst);
    release.send(()).expect("release");
    let outcome = run.join().expect("run thread").expect("run");
    assert_eq!(outcome.notice.as_deref(), Some("Stopped."));

    let first = harness
        .session()
        .submission_by_id(&first.id)
        .expect("read first")
        .expect("first exists");
    let second = harness
        .session()
        .submission_by_id(&second.id)
        .expect("read second")
        .expect("second exists");
    assert_eq!(first.status, SubmissionStatus::Cancelled);
    assert_eq!(second.status, SubmissionStatus::Cancelled);
    assert!(second.answer_entry_id.is_none());
}
