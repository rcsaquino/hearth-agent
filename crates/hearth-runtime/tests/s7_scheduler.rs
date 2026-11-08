use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use hearth_core::{Harness, NewTask};
use hearth_telegram::{SystemSleeper, TelegramClient, TelegramSender};
use hearth_tools::Tool;
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    (home, harness)
}

#[test]
fn the_schedule_tool_creates_a_sleeping_reminder() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "telegram", "42:0")
        .expect("conversation");
    let tool = hearth_runtime::ScheduleTool::new(session.clone(), &conversation.id, 42, None);
    let output = tool
        .call(&json!({"message": "Drink water", "delay_seconds": 60}))
        .expect("schedule");
    assert!(!output.is_error);
    let value: serde_json::Value = serde_json::from_str(&output.content).expect("json");
    let task = session
        .task(value["task_id"].as_str().expect("task id"))
        .expect("read")
        .expect("task");
    assert_eq!(task.kind, "reminder");
    assert_eq!(task.status, hearth_core::TaskStatus::Asleep);
    assert_eq!(task.input["message"], "Drink water");
    assert_eq!(task.input["chat_id"], 42);
}

#[test]
fn the_schedule_tool_rejects_bad_arguments() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "telegram", "42:0")
        .expect("conversation");
    let tool = hearth_runtime::ScheduleTool::new(session.clone(), &conversation.id, 42, None);
    assert!(tool.call(&json!({"delay_seconds": 5})).is_err());
    assert!(
        tool.call(&json!({"message": "x", "delay_seconds": 5, "at": "2030-01-01T00:00:00Z"}))
            .is_err()
    );
}

fn fake_telegram(markdown: Arc<std::sync::Mutex<Vec<String>>>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
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
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        if let Some(text) = value["rich_message"]["markdown"].as_str() {
            markdown.lock().expect("lock").push(text.to_owned());
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
    (base, handle)
}

#[test]
fn the_reminder_handler_sends_once() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "telegram", "42:0")
        .expect("conversation");
    let now = session.now_micros();
    let _task = session
        .create_sleeping_task(
            NewTask::new(
                "reminder",
                1,
                "remind",
                json!({
                    "chat_id": 42,
                    "thread_id": null,
                    "message": "Wake up",
                    "repeat_seconds": null,
                    "first_wake": now - 1
                }),
            )
            .conversation(&conversation.id),
            now - 1,
        )
        .expect("task");
    session.recover_tasks().expect("recovery");
    let claimed = session
        .claim_task_for_kind("scheduler", 60_000_000, "reminder")
        .expect("claim")
        .expect("due");
    let markdown = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (base, server) = fake_telegram(Arc::clone(&markdown));
    let client = TelegramClient::with_base_url("token", base).expect("client");
    let sender = TelegramSender::new(client, Arc::new(SystemSleeper));
    let handler = hearth_runtime::ReminderHandler::new(session.clone(), sender);
    let output = hearth_runtime::TaskHandler::step(&handler, &claimed).expect("step");
    let task = session
        .finish_task(&claimed.id, "scheduler", output)
        .expect("finish");
    server.join().expect("server");
    assert_eq!(task.status, hearth_core::TaskStatus::Done);
    assert_eq!(markdown.lock().expect("lock").as_slice(), ["Wake up"]);
    let effect = session
        .effect_by_key(
            "telegram.send",
            &format!("reminder:{}:{}", claimed.id, now - 1),
        )
        .expect("effect")
        .expect("effect row");
    assert_eq!(effect.status, hearth_core::EffectStatus::Done);
    assert_eq!(task.id, claimed.id);
}

#[test]
fn a_done_reminder_effect_is_not_sent_again() {
    use hearth_core::{EffectIntent, EffectStatus, Save, TaskStatus};
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "telegram", "42:0")
        .expect("conversation");
    let now = session.now_micros();
    let first_wake = now - 1;
    let task = session
        .create_sleeping_task(
            NewTask::new(
                "reminder",
                1,
                "remind",
                json!({
                    "chat_id": 42,
                    "thread_id": null,
                    "message": "Already sent",
                    "repeat_seconds": null,
                    "first_wake": first_wake
                }),
            )
            .conversation(&conversation.id),
            first_wake,
        )
        .expect("task");
    let key = format!("reminder:{}:{}", task.id, first_wake);
    let commit = session
        .save(Save::new(&conversation.id).effect(EffectIntent::new(
            "telegram.send",
            key,
            json!({"reminder": true, "chat_id": 42, "markdown": "Already sent"}),
        )))
        .expect("effect");
    let effect_id = commit.effect_ids[0].clone();
    session.begin_effect(&effect_id).expect("begin");
    session
        .finish_effect(&effect_id, json!({"delivered": true}))
        .expect("finish");
    session.recover_tasks().expect("recovery");
    let claimed = session
        .claim_task_for_kind("scheduler", 60_000_000, "reminder")
        .expect("claim")
        .expect("due");
    let client = TelegramClient::with_base_url("token", "http://127.0.0.1:9").expect("client");
    let sender = TelegramSender::new(client, Arc::new(SystemSleeper));
    let handler = hearth_runtime::ReminderHandler::new(session.clone(), sender);
    let output = hearth_runtime::TaskHandler::step(&handler, &claimed).expect("step");
    assert!(matches!(output, hearth_core::StepOutput::Terminal { .. }));
    let task = session
        .finish_task(&claimed.id, "scheduler", output)
        .expect("finish task");
    assert_eq!(task.status, TaskStatus::Done);
    let effect = session
        .effect_by_key(
            "telegram.send",
            &format!("reminder:{}:{}", task.id, first_wake),
        )
        .expect("effect")
        .expect("row");
    assert_eq!(effect.status, EffectStatus::Done);
}
