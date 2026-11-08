use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use hearth_core::{Harness, SubmissionStatus};
use hearth_model::{MediaError, Transcriber};
use hearth_telegram::{InboundMedia, TelegramClient, TelegramPoller};
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct Request {
    method: String,
    body: Value,
}

#[derive(Default)]
struct State {
    updates: Vec<Value>,
    files: HashMap<String, (String, Vec<u8>, i64)>,
    fail_get_file: bool,
    requests: Vec<Request>,
}

struct FakeBot {
    base_url: String,
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeBot {
    fn start(updates: Vec<Value>, files: Vec<(String, String, Vec<u8>, i64)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake bot");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let state = Arc::new(Mutex::new(State {
            updates,
            files: files
                .into_iter()
                .map(|(file_id, path, bytes, size)| (file_id, (path, bytes, size)))
                .collect(),
            ..State::default()
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = Arc::clone(&state);
            let shutdown = Arc::clone(&shutdown);
            thread::spawn(move || {
                while !shutdown.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => handle_connection(stream, &state),
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
            state,
            shutdown,
            thread: Some(thread),
        }
    }

    fn fail_get_file(&self) {
        self.state.lock().expect("state").fail_get_file = true;
    }

    fn requests(&self) -> Vec<Request> {
        self.state.lock().expect("state").requests.clone()
    }

    fn stop(mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: &Arc<Mutex<State>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return;
        }
        if header == "\r\n" || header == "\n" || header.is_empty() {
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

    if path.starts_with("/file/bot") {
        let file_path = path.splitn(4, '/').nth(3).unwrap_or_default();
        let file_path = file_path.replace("%20", " ");
        let bytes = state
            .lock()
            .expect("state")
            .files
            .values()
            .find(|(file, _, _)| file == &file_path)
            .map(|(_, bytes, _)| bytes.clone());
        let bytes = bytes.unwrap_or_default();
        state.lock().expect("state").requests.push(Request {
            method: "download".to_owned(),
            body: Value::Null,
        });
        write_bytes(&mut stream, &bytes, "application/octet-stream");
        return;
    }

    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let method = path.rsplit('/').next().unwrap_or_default().to_owned();
    let response = {
        let mut state = state.lock().expect("state");
        state.requests.push(Request {
            method: method.clone(),
            body: body.clone(),
        });
        match method.as_str() {
            "getUpdates" => json!({
                "ok": true,
                "result": state.updates.clone(),
            }),
            "getFile" if !state.fail_get_file => {
                let file_id = body["file_id"].as_str().unwrap_or_default();
                match state.files.get(file_id) {
                    Some((file_path, _, size)) => json!({
                        "ok": true,
                        "result": {
                            "file_id": file_id,
                            "file_path": file_path,
                            "file_size": size,
                        }
                    }),
                    None => json!({"ok": false, "error_code": 404, "description": "missing file"}),
                }
            }
            "getFile" => {
                json!({"ok": false, "error_code": 500, "description": "synthetic getFile failure"})
            }
            "setMessageReaction" => json!({"ok": true, "result": true}),
            _ => json!({"ok": true, "result": true}),
        }
    };
    write_json(&mut stream, &response);
}

fn write_json(stream: &mut TcpStream, value: &Value) {
    write_bytes(stream, value.to_string().as_bytes(), "application/json");
}

fn write_bytes(stream: &mut TcpStream, body: &[u8], content_type: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

struct FakeTranscriber {
    text: String,
}

impl Transcriber for FakeTranscriber {
    fn transcribe(&self, _path: &Path) -> Result<String, MediaError> {
        Ok(self.text.clone())
    }
}

fn message(update_id: i64, message_id: i64, body: Value) -> Value {
    let mut message = json!({
        "message_id": message_id,
        "date": 1_700_000_000,
        "chat": {"id": 10, "type": "private"},
        "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
    });
    for (key, value) in body.as_object().expect("body object") {
        message[key] = value.clone();
    }
    json!({"update_id": update_id, "message": message})
}

fn setup(
    bot: &FakeBot,
    transcriber: Option<Arc<dyn Transcriber>>,
) -> (TempDir, Harness, TelegramPoller) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let client = TelegramClient::with_base_url("123:fake", bot.base_url.clone()).expect("client");
    let media = InboundMedia::new(home.path().join("media/inbound"));
    let poller = TelegramPoller::new(client, session, vec![42], vec![], 0, media, transcriber);
    (home, harness, poller)
}

fn submission(poller: &TelegramPoller, request_id: &str) -> hearth_core::Submission {
    let submission = poller
        .session()
        .submission_by_request_id(request_id)
        .expect("read submission")
        .expect("submission exists");
    assert_eq!(submission.status, SubmissionStatus::Queued);
    submission
}

fn blocks(submission: &hearth_core::Submission) -> Vec<Value> {
    submission
        .content
        .as_array()
        .expect("content blocks")
        .clone()
}

#[test]
fn a_photo_is_saved_and_admitted_as_an_image() {
    let bot = FakeBot::start(
        vec![message(
            41,
            5,
            json!({
                "photo": [
                    {"file_id": "small", "file_unique_id": "s", "width": 10, "height": 10, "file_size": 3},
                    {"file_id": "large", "file_unique_id": "l", "width": 100, "height": 100, "file_size": 3}
                ]
            }),
        )],
        vec![(
            "large".to_owned(),
            "photos/large.jpg".to_owned(),
            vec![1, 2, 3],
            3,
        )],
    );
    let (home, _harness, poller) = setup(&bot, None);
    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 1);

    let submission = submission(&poller, "tg:10:5");
    let blocks = blocks(&submission);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "image");
    assert_eq!(blocks[0]["mime"], "image/jpeg");
    let path = Path::new(blocks[0]["path"].as_str().expect("image path"));
    assert!(path.starts_with(home.path().join("media/inbound/2023-11-14")));
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("5-photo.jpg")
    );
    assert_eq!(std::fs::read(path).expect("saved image"), vec![1, 2, 3]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    let requests = bot.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.method == "getFile" && request.body["file_id"] == "large"),
        "requests: {requests:?}"
    );
    assert!(requests.iter().any(|request| request.method == "download"));
    bot.stop();
}

#[test]
fn a_document_is_admitted_as_a_file_block() {
    let bot = FakeBot::start(
        vec![message(
            42,
            6,
            json!({
                "document": {
                    "file_id": "doc",
                    "file_unique_id": "d",
                    "file_name": "report.pdf",
                    "mime_type": "application/pdf",
                    "file_size": 3
                }
            }),
        )],
        vec![(
            "doc".to_owned(),
            "documents/report.pdf".to_owned(),
            vec![4, 5, 6],
            3,
        )],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    poller.poll_once().expect("poll");
    let submission = submission(&poller, "tg:10:6");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["type"], "file");
    assert_eq!(blocks[0]["name"], "report.pdf");
    assert_eq!(blocks[0]["mime"], "application/pdf");
    bot.stop();
}

#[test]
fn a_voice_note_is_transcribed_and_admitted() {
    let bot = FakeBot::start(
        vec![message(
            43,
            7,
            json!({
                "voice": {"file_id": "voice", "file_unique_id": "v", "mime_type": "audio/ogg", "file_size": 3}
            }),
        )],
        vec![(
            "voice".to_owned(),
            "voice/file.oga".to_owned(),
            vec![7, 8, 9],
            3,
        )],
    );
    let transcriber = Arc::new(FakeTranscriber {
        text: "hello from voice".to_owned(),
    });
    let (_home, _harness, poller) = setup(&bot, Some(transcriber));
    poller.poll_once().expect("poll");
    let submission = submission(&poller, "tg:10:7");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "hello from voice");
    assert_eq!(blocks[1]["type"], "file");
    assert_eq!(blocks[1]["name"], "7-voice.ogg");
    assert_eq!(blocks[1]["mime"], "audio/ogg");
    bot.stop();
}

#[test]
fn a_voice_without_a_transcriber_is_admitted_with_a_note() {
    let bot = FakeBot::start(
        vec![message(
            44,
            8,
            json!({
                "voice": {"file_id": "voice", "file_unique_id": "v", "mime_type": "audio/ogg", "file_size": 3}
            }),
        )],
        vec![(
            "voice".to_owned(),
            "voice/file.oga".to_owned(),
            vec![7, 8, 9],
            3,
        )],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    poller.poll_once().expect("poll");
    let submission = submission(&poller, "tg:10:8");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["type"], "text");
    assert!(
        blocks[0]["text"]
            .as_str()
            .expect("note")
            .to_lowercase()
            .contains("not transcribed")
    );
    assert_eq!(blocks[1]["type"], "file");
    bot.stop();
}

#[test]
fn an_attachment_over_twenty_megabytes_is_not_downloaded() {
    let bot = FakeBot::start(
        vec![message(
            45,
            9,
            json!({
                "document": {
                    "file_id": "big",
                    "file_unique_id": "b",
                    "file_name": "big.pdf",
                    "mime_type": "application/pdf",
                    "file_size": 21_000_000
                }
            }),
        )],
        Vec::new(),
    );
    let (_home, _harness, poller) = setup(&bot, None);
    poller.poll_once().expect("poll");
    let submission = submission(&poller, "tg:10:9");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["type"], "text");
    assert!(
        blocks[0]["text"]
            .as_str()
            .expect("note")
            .to_lowercase()
            .contains("too large")
    );
    assert!(
        !bot.requests()
            .iter()
            .any(|request| request.method == "download")
    );
    assert!(
        !bot.requests()
            .iter()
            .any(|request| request.method == "getFile")
    );
    bot.stop();
}

#[test]
fn a_replayed_update_downloads_nothing_and_creates_no_second_submission() {
    let bot = FakeBot::start(
        vec![message(
            46,
            10,
            json!({
                "photo": [
                    {"file_id": "large", "file_unique_id": "l", "width": 100, "height": 100, "file_size": 3}
                ]
            }),
        )],
        vec![(
            "large".to_owned(),
            "photos/large.jpg".to_owned(),
            vec![1, 2, 3],
            3,
        )],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    poller.poll_once().expect("first poll");
    let before = bot
        .requests()
        .iter()
        .filter(|request| request.method == "getFile")
        .count();
    let second = poller.poll_once().expect("second poll");
    assert_eq!(second.duplicates, 1);
    let after = bot
        .requests()
        .iter()
        .filter(|request| request.method == "getFile")
        .count();
    assert_eq!(before, after, "the replay downloaded the file again");
    let _ = submission(&poller, "tg:10:10");
    bot.stop();
}

#[test]
fn a_get_file_failure_still_admits_a_note() {
    let bot = FakeBot::start(
        vec![message(
            47,
            11,
            json!({
                "document": {
                    "file_id": "doc",
                    "file_unique_id": "d",
                    "file_name": "report.pdf",
                    "mime_type": "application/pdf",
                    "file_size": 3
                }
            }),
        )],
        vec![(
            "doc".to_owned(),
            "documents/report.pdf".to_owned(),
            vec![4, 5, 6],
            3,
        )],
    );
    bot.fail_get_file();
    let (_home, _harness, poller) = setup(&bot, None);
    poller.poll_once().expect("poll");
    let submission = submission(&poller, "tg:10:11");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["type"], "text");
    assert!(
        blocks[0]["text"]
            .as_str()
            .expect("note")
            .to_lowercase()
            .contains("could not")
    );
    assert!(
        !bot.requests()
            .iter()
            .any(|request| request.method == "download")
    );
    bot.stop();
}

#[test]
fn text_and_an_attachment_both_appear_in_the_content() {
    let bot = FakeBot::start(
        vec![message(
            48,
            12,
            json!({
                "text": "look at this",
                "photo": [
                    {"file_id": "large", "file_unique_id": "l", "width": 100, "height": 100, "file_size": 3}
                ]
            }),
        )],
        vec![(
            "large".to_owned(),
            "photos/large.jpg".to_owned(),
            vec![1, 2, 3],
            3,
        )],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    poller.poll_once().expect("poll");
    let submission = submission(&poller, "tg:10:12");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "look at this");
    assert_eq!(blocks[1]["type"], "image");
    bot.stop();
}
