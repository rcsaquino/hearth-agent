use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use hearth_core::{Harness, Submission};
use hearth_model::{
    ChatRequest, Provider, ProviderError, ProviderStream, StreamEvent, ToolCall, Usage,
};
use hearth_runtime::skills::{SkillLibrary, SkillTool};
use hearth_runtime::{Agent, AgentConfig};
use hearth_tools::ToolRegistry;
use serde_json::json;
use tempfile::TempDir;

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

fn write_skill(home: &Path) {
    let path = home.join("skills/learned/research.md");
    fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
    fs::write(
        path,
        "---\nname: research\ndescription: Research a topic\nversion: 1\nstatus: active\ncreated: 2026-01-01\nupdated: 2026-01-01\nlast_used: \nuses: 0\nsuccesses: 0\nfailures: 0\nevidence: [entry-1]\n---\n\n## When to use\n\nWhen research is needed.\n\n## Steps\n\nSearch carefully.\n\n## Pitfalls\n\nDo not trust one source.\n",
    )
    .expect("skill file");
}

fn setup() -> (TempDir, Harness, Submission) {
    let home = TempDir::new().expect("temp home");
    write_skill(home.path());
    let harness = Harness::open(home.path()).expect("harness");
    let conversation = harness
        .session()
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");
    let submission = harness
        .session()
        .submit(
            &conversation.id,
            "req-1",
            "input",
            json!({"text": "research this"}),
        )
        .expect("submit")
        .submission;
    (home, harness, submission)
}

fn done() -> Vec<Result<StreamEvent, ProviderError>> {
    vec![
        Ok(StreamEvent::Usage(Usage::default())),
        Ok(StreamEvent::Done {
            finish_reason: Some("stop".to_owned()),
        }),
    ]
}

fn text(message: &hearth_model::ChatMessage) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            hearth_model::ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn the_agent_indexes_skills_and_can_load_one() {
    let (home, harness, submission) = setup();
    let library = Arc::new(SkillLibrary::new(home.path()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        {
            let mut events = vec![Ok(StreamEvent::ToolCall(ToolCall {
                id: "call-1".to_owned(),
                name: "skill".to_owned(),
                arguments: json!({"action": "load", "name": "research"}),
            }))];
            events.extend(done());
            events
        },
        {
            let mut events = vec![Ok(StreamEvent::TextDelta("done".to_owned()))];
            events.extend(done());
            events
        },
    ]));
    let tools = ToolRegistry::new(vec![Arc::new(SkillTool::new(Arc::clone(&library)))]);
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
            tools,
            spill_dir: home.path().join("spill"),
            voice_tags: false,
            guidance: String::new(),
        },
    )
    .with_skills(Arc::clone(&library));

    let outcome = agent.run_submission(&submission.id).expect("run");
    assert_eq!(outcome.answer_text.as_deref(), Some("done"));

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].messages.iter().any(
            |message| text(message).contains("## Skills") && text(message).contains("research")
        ),
        "first request has no skills index"
    );
    assert!(
        requests[0].tools.iter().any(|tool| tool.name == "skill"),
        "skill tool is missing from the request"
    );
    let skill = library
        .find("research")
        .expect("find")
        .expect("skill exists");
    assert_eq!(skill.uses, 1);
    assert!(skill.last_used.is_some());
}
