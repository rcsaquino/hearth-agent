use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use hearth_core::{Harness, Submission};
use hearth_model::{
    ChatRequest, Provider, ProviderError, ProviderStream, StreamEvent, ToolCall, ToolSpec, Usage,
};
use hearth_runtime::{Agent, AgentConfig, ToolGroup, ToolGroups, ToolSearchTool};
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

struct Other;

impl Tool for Other {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "other".to_owned(),
            description: "Another optional tool.".to_owned(),
            parameters: json!({"type": "object"}),
        }
    }

    fn call(&self, _arguments: &Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: "other".to_owned(),
            is_error: false,
        })
    }
}

struct ScriptedProvider {
    scripts: Mutex<VecDeque<Vec<Result<StreamEvent, ProviderError>>>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    fn new(scripts: Vec<Vec<Result<StreamEvent, ProviderError>>>) -> Self {
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
            .expect("a script");
        Ok(Box::new(script.into_iter()))
    }
}

fn setup() -> (TempDir, Harness, Submission) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let submission = harness
        .session()
        .submit(&conversation.id, "req-1", "input", json!({"text": "hi"}))
        .expect("submit")
        .submission;
    (home, harness, submission)
}

fn call(id: &str, name: &str, arguments: Value) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::ToolCall(ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    }))
}

fn done() -> Vec<Result<StreamEvent, ProviderError>> {
    vec![
        Ok(StreamEvent::Usage(Usage::default())),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ]
}

#[test]
fn tool_search_loads_a_group_for_the_next_request() {
    let (home, harness, submission) = setup();
    let groups = Arc::new(ToolGroups::new(vec![
        ToolGroup::new("test", "A test group.", vec![Arc::new(Echo)]),
        ToolGroup::new("other", "Another group.", vec![Arc::new(Other)]),
    ]));
    let selection = Arc::new(Mutex::new(HashSet::new()));
    let core = ToolRegistry::new(vec![Arc::new(ToolSearchTool::new(
        Arc::clone(&groups),
        Arc::clone(&selection),
    ))]);
    let provider = Arc::new(ScriptedProvider::new(vec![
        {
            let mut events = vec![call("c1", "tool_search", json!({"select": "test"}))];
            events.extend(done());
            events
        },
        {
            let mut events = vec![call("c2", "echo", json!({"text": "loaded"}))];
            events.extend(done());
            events
        },
        {
            let mut events = vec![Ok(StreamEvent::TextDelta("done".to_owned()))];
            events.extend(done());
            events
        },
    ]));
    let agent = Agent::new(
        harness.session().clone(),
        provider.clone(),
        hearth_model::Pricing::default(),
        AgentConfig {
            soul: "test soul".to_owned(),
            model: "test-model".to_owned(),
            provider_name: "scripted".to_owned(),
            owner: "owner".to_owned(),
            max_turns: 6,
            tools: core,
            spill_dir: home.path().join("spill"),
            voice_tags: false,
        },
    )
    .with_groups(Arc::clone(&groups), Arc::clone(&selection));

    let outcome = agent.run_submission(&submission.id).expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("done"));

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    let names = |request: &ChatRequest| {
        request
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>()
    };
    assert!(names(&requests[0]).contains(&"tool_search".to_owned()));
    assert!(!names(&requests[0]).contains(&"echo".to_owned()));
    assert!(!names(&requests[0]).contains(&"other".to_owned()));
    assert!(names(&requests[1]).contains(&"echo".to_owned()));
    assert!(!names(&requests[1]).contains(&"other".to_owned()));
    assert!(names(&requests[2]).contains(&"echo".to_owned()));

    let task = harness
        .session()
        .task(&outcome.task_id)
        .expect("task")
        .expect("task exists");
    let selected = task
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.get("selected_groups"))
        .and_then(Value::as_array)
        .expect("selected groups in the checkpoint");
    assert_eq!(selected, &vec![json!("test")]);

    let answer = harness
        .session()
        .submission_by_id(&submission.id)
        .expect("read")
        .expect("submission");
    assert_eq!(answer.status, hearth_core::SubmissionStatus::Done);
}
