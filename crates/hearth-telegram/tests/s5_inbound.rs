use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use hearth_core::{Clock, Harness, IdGen, SubmissionStatus};
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

    fn push_update(&self, update: Value) {
        self.state.lock().expect("state").updates.push(update);
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
    let (home, harness, poller, _clock) = setup_with_clock(bot, transcriber);
    (home, harness, poller)
}

struct TestClock(Arc<AtomicI64>);

impl Clock for TestClock {
    fn now_micros(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct TestIds(Arc<AtomicU64>);

impl IdGen for TestIds {
    fn new_id(&self) -> String {
        format!("id-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

fn setup_with_clock(
    bot: &FakeBot,
    transcriber: Option<Arc<dyn Transcriber>>,
) -> (TempDir, Harness, TelegramPoller, Arc<AtomicI64>) {
    let home = TempDir::new().expect("temp home");
    let clock = Arc::new(AtomicI64::new(1_000_000));
    let ids = Arc::new(AtomicU64::new(0));
    let harness = Harness::open_with(
        home.path(),
        Arc::new(TestClock(Arc::clone(&clock))),
        Arc::new(TestIds(ids)),
    )
    .expect("open harness");
    let session = harness.session().clone();
    let client = TelegramClient::with_base_url("123:fake", bot.base_url.clone()).expect("client");
    let media = InboundMedia::new(home.path().join("media/inbound"));
    let poller = TelegramPoller::new(
        client,
        session,
        vec![42],
        vec![],
        0,
        media,
        transcriber,
        None,
    );
    (home, harness, poller, clock)
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

#[test]
fn a_reply_puts_the_quoted_text_first_in_the_content() {
    let bot = FakeBot::start(
        vec![message(
            49,
            13,
            json!({
                "text": "what does this mean",
                "reply_to_message": {
                    "message_id": 7,
                    "date": 1_700_000_000,
                    "chat": {"id": 10, "type": "private"},
                    "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                    "text": "the case report is due Friday",
                },
            }),
        )],
        vec![],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 1);

    let blocks = blocks(&submission(&poller, "tg:10:13"));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    let context = blocks[0]["text"].as_str().expect("context text");
    assert!(context.contains("Replying to"), "{context}");
    assert!(
        context.contains("the case report is due Friday"),
        "{context}"
    );
    assert_eq!(blocks[1]["type"], "text");
    assert_eq!(blocks[1]["text"], "what does this mean");
    bot.stop();
}

#[test]
fn a_reply_to_a_media_only_message_is_described_and_still_admitted() {
    let bot = FakeBot::start(
        vec![message(
            50,
            14,
            json!({
                "photo": [
                    {"file_id": "large", "file_unique_id": "l", "width": 100, "height": 100, "file_size": 3}
                ],
                "reply_to_message": {
                    "message_id": 6,
                    "date": 1_700_000_000,
                    "chat": {"id": 10, "type": "private"},
                    "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                    "voice": {"file_id": "voice", "file_unique_id": "v", "file_size": 3},
                },
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
    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 1);

    let blocks = blocks(&submission(&poller, "tg:10:14"));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "Replying to Sam: (a voice message)");
    assert_eq!(blocks[1]["type"], "image");
    bot.stop();
}

fn album_member(update_id: i64, message_id: i64, caption: Option<&str>, file_id: &str) -> Value {
    let mut body = json!({
        "media_group_id": "album-1",
        "photo": [
            {"file_id": file_id, "file_unique_id": file_id, "width": 100, "height": 100, "file_size": 3}
        ],
    });
    if let Some(caption) = caption {
        body["caption"] = json!(caption);
    }
    message(update_id, message_id, body)
}

fn album_files(prefix: &str) -> Vec<(String, String, Vec<u8>, i64)> {
    vec![
        (
            format!("{prefix}-a"),
            "photos/a.jpg".to_owned(),
            vec![1, 2, 3],
            3,
        ),
        (
            format!("{prefix}-b"),
            "photos/b.jpg".to_owned(),
            vec![4, 5, 6],
            3,
        ),
    ]
}

fn image_paths(blocks: &[Value]) -> Vec<String> {
    blocks
        .iter()
        .filter(|block| block["type"] == "image")
        .map(|block| block["path"].as_str().expect("image path").to_owned())
        .collect()
}

const ALBUM_QUIET_MICROS: i64 = 1_000_000;

#[test]
fn an_album_is_staged_without_starting_a_turn() {
    let first = album_member(81, 30, None, "album-a");
    let second = album_member(82, 31, Some("Does this warrant ERCP?"), "album-b");
    let bot = FakeBot::start(vec![first, second], album_files("album"));
    let (_home, _harness, poller, _clock) = setup_with_clock(&bot, None);
    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 0, "the album waits for its quiet window");
    assert!(
        poller
            .session()
            .submission_by_request_id("tg:10:30")
            .expect("lookup")
            .is_none(),
        "no turn starts before the album is due"
    );
    bot.stop();
}

#[test]
fn an_album_is_admitted_after_its_quiet_window() {
    let first = album_member(83, 32, None, "quiet-a");
    let second = album_member(84, 33, Some("Does this warrant ERCP?"), "quiet-b");
    let bot = FakeBot::start(vec![first, second], album_files("quiet"));
    let (_home, _harness, poller, clock) = setup_with_clock(&bot, None);
    let first_report = poller.poll_once().expect("first poll");
    assert_eq!(first_report.admitted, 0);

    clock.fetch_add(ALBUM_QUIET_MICROS + 1, Ordering::SeqCst);
    let report = poller.poll_once().expect("second poll");
    assert_eq!(report.admitted, 1, "the due album starts one turn");

    let submission = submission(&poller, "tg:10:32");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["text"], "Does this warrant ERCP?");
    let images = image_paths(&blocks);
    assert_eq!(
        images.len(),
        2,
        "every photo belongs to the album: {blocks:?}"
    );
    assert!(images[0].ends_with("32-photo.jpg"), "{}", images[0]);
    assert!(images[1].ends_with("33-photo.jpg"), "{}", images[1]);
    bot.stop();
}

#[test]
fn a_later_album_member_extends_the_quiet_window() {
    let first = album_member(85, 34, None, "extend-a");
    let bot = FakeBot::start(vec![first], album_files("extend"));
    let (_home, _harness, poller, clock) = setup_with_clock(&bot, None);
    let report = poller.poll_once().expect("first poll");
    assert_eq!(report.admitted, 0);

    clock.fetch_add(ALBUM_QUIET_MICROS - 400_000, Ordering::SeqCst);
    bot.push_update(album_member(
        86,
        35,
        Some("Does this warrant ERCP?"),
        "extend-b",
    ));
    let report = poller.poll_once().expect("second poll");
    assert_eq!(report.admitted, 0);

    clock.fetch_add(500_000, Ordering::SeqCst);
    let report = poller.poll_once().expect("third poll");
    assert_eq!(report.admitted, 0, "the second member moved the window");

    clock.fetch_add(600_000, Ordering::SeqCst);
    let report = poller.poll_once().expect("fourth poll");
    assert_eq!(report.admitted, 1);

    let submission = submission(&poller, "tg:10:34");
    let blocks = blocks(&submission);
    assert_eq!(blocks[0]["text"], "Does this warrant ERCP?");
    assert_eq!(image_paths(&blocks).len(), 2);
    bot.stop();
}

#[test]
fn a_repeated_album_caption_appears_once() {
    let first = album_member(87, 36, Some("Same caption"), "same-a");
    let second = album_member(88, 37, Some("Same caption"), "same-b");
    let bot = FakeBot::start(vec![first, second], album_files("same"));
    let (_home, _harness, poller, clock) = setup_with_clock(&bot, None);
    poller.poll_once().expect("first poll");
    clock.fetch_add(ALBUM_QUIET_MICROS + 1, Ordering::SeqCst);
    poller.poll_once().expect("second poll");

    let submission = submission(&poller, "tg:10:36");
    let blocks = blocks(&submission);
    let texts: Vec<&Value> = blocks
        .iter()
        .filter(|block| block["type"] == "text")
        .collect();
    assert_eq!(
        image_paths(&blocks).len(),
        2,
        "both members are one submission"
    );
    assert_eq!(texts.len(), 1, "one caption block: {blocks:?}");
    assert_eq!(texts[0]["text"], "Same caption");
    bot.stop();
}

#[test]
fn one_undecodable_update_does_not_stop_the_batch() {
    let bot = FakeBot::start(
        vec![
            message(61, 1, json!({"text": "the first message"})),
            json!({"update_id": 62, "message": {"message_id": 2}}),
        ],
        vec![],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    let report = poller.poll_once().expect("poll");

    assert_eq!(
        report.admitted, 1,
        "the decodable update in the batch must still be admitted"
    );
    submission(&poller, "tg:10:1");
    assert_eq!(poller.session().telegram_offset().expect("offset"), 63);
}

#[test]
fn a_reply_to_a_rich_message_keeps_the_quote() {
    let bot = FakeBot::start(
        vec![message(
            80,
            20,
            json!({
                "text": "why did you send this?",
                "reply_to_message": {
                    "message_id": 7,
                    "date": 1_700_000_000,
                    "chat": {"id": 10, "type": "private"},
                    "from": {"id": 999, "is_bot": true, "first_name": "Hearth"},
                    "rich_message": {
                        "blocks": [
                            {"type": "paragraph", "text": "The errands are due Friday."},
                            {"type": "table", "cells": [[{"text": "Task"}, {"text": "When"}]]}
                        ]
                    }
                },
            }),
        )],
        vec![],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    let report = poller.poll_once().expect("poll");
    assert_eq!(report.admitted, 1);

    let blocks = blocks(&submission(&poller, "tg:10:20"));
    assert_eq!(blocks.len(), 2);
    let context = blocks[0]["text"].as_str().expect("context text");
    assert!(context.contains("Replying to Hearth"), "{context}");
    assert!(context.contains("The errands are due Friday."), "{context}");
    assert_eq!(blocks[1]["text"], "why did you send this?");
    bot.stop();
}

#[test]
fn a_quoted_span_arrives_as_reply_context() {
    let bot = FakeBot::start(
        vec![message(
            71,
            1,
            json!({
                "text": "what does this reply to?",
                "external_reply": {
                    "quote": {"text": "the highlighted span", "is_manual": true}
                }
            }),
        )],
        vec![],
    );
    let (_home, _harness, poller) = setup(&bot, None);
    poller.poll_once().expect("poll");

    let submission = submission(&poller, "tg:10:1");
    let blocks = blocks(&submission);
    assert!(
        blocks.iter().any(|block| block["text"]
            .as_str()
            .is_some_and(|text| text.contains("the highlighted span"))),
        "the quoted span must reach the model: {blocks:?}"
    );
}
