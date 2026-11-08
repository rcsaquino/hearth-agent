use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use hearth_core::{Harness, SubmissionStatus};
use hearth_telegram::{InboundMedia, TelegramClient, TelegramPoller};
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct Request {
    method: String,
    body: Value,
}

struct FakeApi {
    base_url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    reaction_fails: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeApi {
    fn start(updates: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake api");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let updates = Arc::new(Mutex::new(updates));
        let reaction_fails = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread = {
            let requests = Arc::clone(&requests);
            let updates = Arc::clone(&updates);
            let reaction_fails = Arc::clone(&reaction_fails);
            let shutdown = Arc::clone(&shutdown);
            thread::spawn(move || {
                while !shutdown.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => handle_connection(
                            stream,
                            &requests,
                            &updates,
                            &reaction_fails,
                            &shutdown,
                        ),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            base_url: format!("http://{address}"),
            requests,
            reaction_fails,
            shutdown,
            thread: Some(thread),
        }
    }

    fn fail_reactions(&self, fail: bool) {
        self.reaction_fails.store(fail, Ordering::Relaxed);
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("requests").clone()
    }

    fn stop(mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_connection(
    stream: TcpStream,
    requests: &Arc<Mutex<Vec<Request>>>,
    updates: &Arc<Mutex<Vec<Value>>>,
    reaction_fails: &Arc<AtomicBool>,
    shutdown: &Arc<AtomicBool>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        match reader.read_line(&mut request_line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
            Err(_) => break,
        }
        if request_line.trim().is_empty() {
            continue;
        }
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let mut content_length = 0usize;
        loop {
            let mut header = String::new();
            match reader.read_line(&mut header) {
                Ok(0) => return,
                Ok(_) => {}
                Err(_) => return,
            }
            if header == "\r\n" || header == "\n" {
                break;
            }
            if header.to_ascii_lowercase().starts_with("content-length:") {
                content_length = header
                    .split(':')
                    .nth(1)
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(0);
            }
        }
        let mut body = vec![0u8; content_length];
        if content_length > 0 && reader.read_exact(&mut body).is_err() {
            return;
        }
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let method = path.rsplit('/').next().unwrap_or_default().to_owned();
        requests.lock().expect("requests").push(Request {
            method: method.clone(),
            body: body.clone(),
        });

        let response = if method == "getUpdates" {
            let updates = updates.lock().expect("updates").clone();
            json!({"ok": true, "result": updates})
        } else if method == "setMessageReaction" {
            if reaction_fails.load(Ordering::Relaxed) {
                json!({"ok": false, "error_code": 500, "description": "synthetic reaction failure"})
            } else {
                json!({"ok": true, "result": true})
            }
        } else {
            json!({"ok": true, "result": true})
        };
        let text = response.to_string();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            text.len()
        );
        if writer.write_all(head.as_bytes()).is_err()
            || writer.write_all(text.as_bytes()).is_err()
            || writer.flush().is_err()
        {
            break;
        }
    }
}

fn update(update_id: i64, chat_id: i64, message_id: i64, user_id: i64, text: Value) -> Value {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": message_id,
            "chat": {"id": chat_id, "type": "private"},
            "from": {"id": user_id, "is_bot": false, "first_name": "Sam"},
            "text": text,
        }
    })
}

fn setup(
    api: &FakeApi,
    allowed_users: Vec<i64>,
    allowed_chats: Vec<i64>,
) -> (TempDir, Harness, TelegramPoller) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let client =
        TelegramClient::with_base_url("123:fake", api.base_url.clone()).expect("telegram client");
    let media = InboundMedia::new(home.path().join("media/inbound"));
    let poller = TelegramPoller::new(
        client,
        session,
        allowed_users,
        allowed_chats,
        0,
        media,
        None,
    );
    (home, harness, poller)
}

#[test]
fn an_authorized_text_update_is_admitted() {
    let api = FakeApi::start(vec![update(42, 1, 10, 42, json!("hello"))]);
    let (_home, harness, poller) = setup(&api, vec![42], vec![]);

    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 1);
    assert_eq!(report.duplicates, 0);
    assert_eq!(report.next_offset, 43);

    let session = harness.session();
    let conversation = session
        .conversation_by_transport("telegram", "1:0")
        .expect("lookup")
        .expect("conversation");
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[0].source.as_deref(), Some("10"));
    assert_eq!(history[0].content[0]["text"], "hello");

    let submission = session
        .submission_by_request_id("tg:1:10")
        .expect("submission")
        .expect("submission exists");
    assert_eq!(submission.status, SubmissionStatus::Queued);
    assert_eq!(session.telegram_offset().expect("offset"), 43);
    assert!(session.telegram_update_seen(42).expect("seen"));

    let requests = api.requests();
    let methods: Vec<&str> = requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    assert_eq!(methods, vec!["getUpdates", "setMessageReaction"]);
    assert_eq!(requests[0].body["offset"], 0);
    assert_eq!(requests[0].body["allowed_updates"][0], "message");
    assert_eq!(requests[1].body["chat_id"], 1);
    assert_eq!(requests[1].body["message_id"], 10);
    assert_eq!(requests[1].body["reaction"][0]["emoji"], "👀");

    drop(poller);
    api.stop();
}

#[test]
fn get_updates_sends_the_stored_offset_and_allowed_updates() {
    let api = FakeApi::start(vec![]);
    let (_home, harness, poller) = setup(&api, vec![42], vec![]);
    harness
        .session()
        .set_telegram_offset(41)
        .expect("set offset");

    let report = poller.poll_once().expect("poll");
    assert_eq!(report.next_offset, 41);
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "getUpdates");
    assert_eq!(requests[0].body["offset"], 41);
    assert_eq!(requests[0].body["timeout"], 0);
    assert_eq!(
        requests[0].body["allowed_updates"],
        json!(["message", "callback_query"])
    );

    drop(poller);
    api.stop();
}

#[test]
fn a_second_message_reuses_the_conversation() {
    let api = FakeApi::start(vec![
        update(1, 1, 10, 42, json!("first")),
        update(2, 1, 11, 42, json!("second")),
    ]);
    let (_home, harness, poller) = setup(&api, vec![42], vec![]);

    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 2);
    let session = harness.session();
    let first = session
        .submission_by_request_id("tg:1:10")
        .expect("submission")
        .expect("first");
    let second = session
        .submission_by_request_id("tg:1:11")
        .expect("submission")
        .expect("second");
    assert_eq!(first.conversation_id, second.conversation_id);
    assert_eq!(
        session
            .history(&first.conversation_id)
            .expect("history")
            .len(),
        2
    );

    drop(poller);
    api.stop();
}

#[test]
fn a_replayed_update_is_not_admitted_twice() {
    let api = FakeApi::start(vec![update(42, 1, 10, 42, json!("hello"))]);
    let (_home, harness, poller) = setup(&api, vec![42], vec![]);

    let first = poller.poll_once().expect("first poll");
    assert_eq!(first.admitted, 1);
    let second = poller.poll_once().expect("second poll");
    assert_eq!(second.duplicates, 1);
    assert_eq!(second.next_offset, 43);

    let session = harness.session();
    assert_eq!(session.telegram_offset().expect("offset"), 43);
    let conversation = session
        .conversation_by_transport("telegram", "1:0")
        .expect("lookup")
        .expect("conversation");
    assert_eq!(session.history(&conversation.id).expect("history").len(), 1);
    let reaction_count = api
        .requests()
        .iter()
        .filter(|request| request.method == "setMessageReaction")
        .count();
    assert_eq!(reaction_count, 1);
    let offsets: Vec<i64> = api
        .requests()
        .iter()
        .filter(|request| request.method == "getUpdates")
        .map(|request| request.body["offset"].as_i64().unwrap_or(-1))
        .collect();
    assert_eq!(offsets, vec![0, 43]);

    drop(poller);
    api.stop();
}

#[test]
fn an_unauthorized_sender_is_ignored() {
    let api = FakeApi::start(vec![update(42, 1, 10, 7, json!("secret"))]);
    let (_home, harness, poller) = setup(&api, vec![42], vec![]);

    let report = poller.poll_once().expect("poll");
    assert_eq!(report.ignored, 1);
    let session = harness.session();
    assert_eq!(session.telegram_offset().expect("offset"), 43);
    assert!(
        session
            .conversation_by_transport("telegram", "1:0")
            .expect("lookup")
            .is_none()
    );
    assert!(
        api.requests()
            .iter()
            .all(|request| request.method != "setMessageReaction")
    );

    drop(poller);
    api.stop();
}

#[test]
fn an_unauthorized_chat_is_ignored() {
    let api = FakeApi::start(vec![update(42, 1, 10, 42, json!("hello"))]);
    let (_home, harness, poller) = setup(&api, vec![42], vec![999]);

    let report = poller.poll_once().expect("poll");
    assert_eq!(report.ignored, 1);
    assert_eq!(harness.session().telegram_offset().expect("offset"), 43);
    assert!(
        harness
            .session()
            .conversation_by_transport("telegram", "1:0")
            .expect("lookup")
            .is_none()
    );

    drop(poller);
    api.stop();
}

#[test]
fn a_non_text_update_is_ignored() {
    let api = FakeApi::start(vec![json!({
        "update_id": 42,
        "message": {
            "message_id": 10,
            "chat": {"id": 1, "type": "private"},
            "from": {"id": 42, "is_bot": false, "first_name": "Sam"}
        }
    })]);
    let (_home, harness, poller) = setup(&api, vec![42], vec![]);

    let report = poller.poll_once().expect("poll");
    assert_eq!(report.ignored, 1);
    assert_eq!(harness.session().telegram_offset().expect("offset"), 43);
    assert!(
        harness
            .session()
            .conversation_by_transport("telegram", "1:0")
            .expect("lookup")
            .is_none()
    );

    drop(poller);
    api.stop();
}

#[test]
fn a_reaction_failure_does_not_block_admission() {
    let api = FakeApi::start(vec![update(42, 1, 10, 42, json!("hello"))]);
    api.fail_reactions(true);
    let (_home, harness, poller) = setup(&api, vec![42], vec![]);

    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 1);
    assert!(
        harness
            .session()
            .submission_by_request_id("tg:1:10")
            .expect("submission")
            .is_some()
    );

    drop(poller);
    api.stop();
}
