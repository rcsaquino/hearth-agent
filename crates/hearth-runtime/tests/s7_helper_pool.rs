use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use hearth_config::Settings;
use hearth_core::{ContextBudget, Harness};
use hearth_memory::MemoryService;
use hearth_model::{
    ChatRequest, Pricing, Provider, ProviderError, ProviderStream, StreamEvent,
};
use hearth_runtime::{
    HelperConfig, HelperControls, HelperPool, NoopSink, TaskTool,
};
use hearth_telegram::{SystemSleeper, TelegramClient, TelegramSender};
use hearth_tools::Tool;
use serde_json::{Value, json};
use tempfile::TempDir;

struct ScriptedProvider {
    answer: String,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    fn new(answer: &str) -> Self {
        Self {
            answer: answer.to_owned(),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl Provider for ScriptedProvider {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().expect("requests").push(request);
        let events = vec![
            Ok(StreamEvent::TextDelta(self.answer.clone())),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ];
        Ok(Box::new(events.into_iter()))
    }
}

fn settings() -> Settings {
    serde_json::from_value(json!({
        "identity": {"agent_name": "Hearth", "user_name": "Sam", "timezone": "UTC"},
        "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
        "models": {
            "kind": "openai-compatible",
            "base_url": "http://127.0.0.1:1/v1",
            "api_key": "fake",
            "model": "fake-model"
        }
    }))
    .expect("settings")
}

fn fake_telegram() -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let sent = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::clone(&sent);
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("line");
        let mut length = 0usize;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).expect("header");
            if header == "\r\n" || header == "\n" || header.is_empty() {
                break;
            }
            if header.to_ascii_lowercase().starts_with("content-length:") {
                length = header
                    .split(':')
                    .nth(1)
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        if let Some(text) = value["rich_message"]["markdown"].as_str() {
            state.lock().expect("sent").push(text.to_owned());
        }
        let response = r#"{"ok":true,"result":{"message_id":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("response");
    });
    (base, sent, handle)
}

#[test]
fn a_helper_runs_and_reports_without_the_parent_transcript() {
    let home = TempDir::new().expect("home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let parent = session
        .create_conversation("Parent", "telegram", "42:0")
        .expect("parent");
    session
        .submit(&parent.id, "parent-1", "input", json!({"text": "Parent secret context"}))
        .expect("parent input");

    let controls = Arc::new(HelperControls::new());
    let tool = TaskTool::new(
        session.clone(),
        &parent.id,
        42,
        None,
        0,
        Arc::clone(&controls),
        2,
        4,
    );
    let spawn = tool
        .call(&json!({
            "action": "spawn",
            "brief": "Summarize the helper job and return a report.",
            "expected_minutes": 10
        }))
        .expect("spawn");
    let spawn: Value = serde_json::from_str(&spawn.content).expect("spawn json");
    let task_id = spawn["task_id"].as_str().expect("task id").to_owned();

    let (base, sent, telegram) = fake_telegram();
    let sender = TelegramSender::new(
        TelegramClient::with_base_url("123:fake", &base).expect("client"),
        Arc::new(SystemSleeper),
    );
    let provider = Arc::new(ScriptedProvider::new("Helper report: done."));
    let memory = Arc::new(MemoryService::new(session.clone()));
    let config = Arc::new(HelperConfig {
        settings: Arc::new(settings()),
        session: session.clone(),
        provider: Arc::clone(&provider) as Arc<dyn Provider>,
        sender,
        speaker: None,
        memory,
        soul: "You are Hearth.".to_owned(),
        workspace: home.path().join("workspace"),
        spill: home.path().join("spill"),
        outbound: home.path().join("media/outbound"),
        model: "fake-model".to_owned(),
        provider_name: "openai-compatible".to_owned(),
        max_turns: 10,
        budget: ContextBudget::default(),
        pricing: Pricing::new(),
        events: Arc::new(NoopSink),
        owner: "helper-test".to_owned(),
    });
    let pool = HelperPool::new(config, controls);
    assert!(pool.run_once().expect("run"));

    let task = session.task(&task_id).expect("task").expect("exists");
    assert_eq!(task.status, hearth_core::TaskStatus::Done);
    let history = session.history(&parent.id).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].kind, "assistant");
    assert_eq!(history[1].content["text"], "Helper report: done.");
    assert_eq!(history[1].source.as_deref(), Some(format!("helper:{task_id}").as_str()));

    let report_path = home.path().join("spill").join(format!("helper-{task_id}.txt"));
    assert_eq!(
        std::fs::read_to_string(&report_path).expect("report file"),
        "Helper report: done."
    );

    let notices = sent.lock().expect("notices").clone();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("Helper report: done."));
    assert!(notices[0].contains(&report_path.display().to_string()));

    let requests = provider.requests.lock().expect("requests").clone();
    assert_eq!(requests.len(), 1);
    let prompt = format!("{:?}", requests[0].messages);
    assert!(prompt.contains("Summarize the helper job"));
    assert!(!prompt.contains("Parent secret context"));
    telegram.join().expect("telegram");
}

#[test]
fn a_missing_report_file_is_written_with_private_mode() {
    let home = TempDir::new().expect("home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let parent = session
        .create_conversation("Parent", "telegram", "42:0")
        .expect("parent");
    let controls = Arc::new(HelperControls::new());
    let tool = TaskTool::new(session.clone(), &parent.id, 42, None, 0, controls.clone(), 2, 4);
    let spawn = tool
        .call(&json!({"action": "spawn", "brief": "Write a report"}))
        .expect("spawn");
    let spawn: Value = serde_json::from_str(&spawn.content).expect("json");
    let task_id = spawn["task_id"].as_str().expect("task id").to_owned();
    let (base, _sent, telegram) = fake_telegram();
    let sender = TelegramSender::new(
        TelegramClient::with_base_url("123:fake", &base).expect("client"),
        Arc::new(SystemSleeper),
    );
    let provider = Arc::new(ScriptedProvider::new("Report."));
    let memory = Arc::new(MemoryService::new(session.clone()));
    let config = Arc::new(HelperConfig {
        settings: Arc::new(settings()),
        session: session.clone(),
        provider: Arc::clone(&provider) as Arc<dyn Provider>,
        sender,
        speaker: None,
        memory,
        soul: "You are Hearth.".to_owned(),
        workspace: home.path().join("workspace"),
        spill: home.path().join("spill"),
        outbound: home.path().join("media/outbound"),
        model: "fake-model".to_owned(),
        provider_name: "openai-compatible".to_owned(),
        max_turns: 10,
        budget: ContextBudget::default(),
        pricing: Pricing::new(),
        events: Arc::new(NoopSink),
        owner: "helper-test".to_owned(),
    });
    let pool = HelperPool::new(config, controls);
    assert!(pool.run_once().expect("run"));
    let path = home.path().join("spill").join(format!("helper-{task_id}.txt"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    telegram.join().expect("telegram");
}
