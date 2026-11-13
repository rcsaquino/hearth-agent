use std::sync::{Arc, Mutex};

use hearth_core::Harness;
use hearth_model::{
    ChatRequest, Provider, ProviderError, ProviderStream, StreamEvent, ToolCall, ToolSpec,
};
use hearth_runtime::{Agent, AgentConfig};
use hearth_tools::{Tool, ToolError, ToolOutput, ToolRegistry};
use serde_json::{Value, json};
use tempfile::TempDir;

struct LeakyTool;

impl Tool for LeakyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "leaky".to_owned(),
            description: "return a large credential-shaped output".to_owned(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    fn call(&self, _arguments: &Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: format!("prefix sk-test-not-a-real-key {}", "filler ".repeat(3000)),
            is_error: false,
        })
    }
}

struct Scripted {
    calls: Mutex<usize>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl Scripted {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(0),
            requests: Mutex::new(Vec::new()),
        })
    }
}

impl Provider for Scripted {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().expect("requests").push(request);
        let mut calls = self.calls.lock().expect("calls");
        *calls += 1;
        let events = if *calls == 1 {
            vec![
                Ok(StreamEvent::ToolCall(ToolCall {
                    id: "call-leaky".to_owned(),
                    name: "leaky".to_owned(),
                    arguments: json!({}),
                })),
                Ok(StreamEvent::Done {
                    finish_reason: Some("tool_calls".to_owned()),
                }),
            ]
        } else {
            vec![
                Ok(StreamEvent::TextDelta("done".to_owned())),
                Ok(StreamEvent::Done {
                    finish_reason: Some("stop".to_owned()),
                }),
            ]
        };
        Ok(Box::new(events.into_iter()))
    }
}

struct FailingProvider;

impl Provider for FailingProvider {
    fn stream(&self, _request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        Err(ProviderError::Request {
            status: 500,
            message: "provider failed with token=abcdefghijk".to_owned(),
            retryable: false,
        })
    }
}

fn home() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

fn config(home: &std::path::Path, tools: ToolRegistry) -> AgentConfig {
    AgentConfig {
        soul: "Test identity.".to_owned(),
        model: "test-model".to_owned(),
        provider_name: "scripted".to_owned(),
        owner: "test-owner".to_owned(),
        tools,
        spill_dir: home.join("spill"),
        guidance: String::new(),
    }
}

#[test]
fn tool_output_and_spill_do_not_store_credentials() {
    let (home, harness) = home();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let submitted = session
        .submit(
            &conversation.id,
            "req-secret-tool",
            "input",
            json!({"text": "check"}),
        )
        .expect("submit");
    let provider = Scripted::new();
    let agent = Agent::new(
        session.clone(),
        provider.clone(),
        config(home.path(), ToolRegistry::new(vec![Arc::new(LeakyTool)])),
    );
    let outcome = agent.run_submission(&submitted.submission.id).expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("done"));

    let history = session.history(&conversation.id).expect("history");
    let record = history
        .iter()
        .map(|entry| entry.content.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !record.contains("sk-test-not-a-real-key"),
        "entry leaked: {record}"
    );
    assert!(record.contains("[redacted]"));

    let spill: Vec<String> = std::fs::read_dir(home.path().join("spill"))
        .expect("spill dir")
        .map(|entry| std::fs::read_to_string(entry.expect("entry").path()).expect("spill file"))
        .collect();
    assert!(!spill.is_empty());
    for text in &spill {
        assert!(!text.contains("sk-test-not-a-real-key"), "spill leaked");
        assert!(text.contains("[redacted]"));
    }

    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        for message in &request.messages {
            for block in &message.blocks {
                if let hearth_model::ContentBlock::ToolResult { content, .. } = block {
                    assert!(!content.contains("sk-test-not-a-real-key"));
                }
            }
        }
    }
}

#[test]
fn provider_errors_are_redacted_in_the_notice_and_transcript() {
    let (_home, harness) = home();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let submitted = session
        .submit(
            &conversation.id,
            "req-secret-error",
            "input",
            json!({"text": "hello"}),
        )
        .expect("submit");
    let agent = Agent::new(
        session.clone(),
        Arc::new(FailingProvider),
        config(_home.path(), ToolRegistry::new(vec![])),
    );
    let outcome = agent.run_submission(&submitted.submission.id).expect("run");
    let notice = outcome.notice.expect("notice");
    assert!(!notice.contains("abcdefghijk"));
    let history = session.history(&conversation.id).expect("history");
    let record = history
        .iter()
        .map(|entry| entry.content.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!record.contains("abcdefghijk"));
    assert!(record.contains("[redacted]"));
}
