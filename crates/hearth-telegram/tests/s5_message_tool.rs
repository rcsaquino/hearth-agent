use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use hearth_model::{Speaker, SpeechClient};
use hearth_telegram::{MessageTool, TelegramClient, TelegramSender};
use hearth_tools::Tool;
use serde_json::json;
use tempfile::TempDir;

struct Reply {
    body: serde_json::Value,
}

struct Recorded {
    path: String,
    body: Vec<u8>,
}

fn bot_server(replies: Vec<Reply>) -> (String, thread::JoinHandle<Vec<Recorded>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake bot api");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_request(&mut stream);
            requests.push(request);
            let body = reply.body.to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write response");
        }
        requests
    });
    (base, handle)
}

fn speech_server(bytes: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake elevenlabs");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept speech");
        let _ = read_request(&mut stream);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: audio/ogg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        stream.write_all(head.as_bytes()).expect("head");
        stream.write_all(&bytes).expect("audio");
        stream.flush().expect("flush");
    });
    (base, handle)
}

fn read_request(stream: &mut TcpStream) -> Recorded {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("request line");
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).expect("header");
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
    reader.read_exact(&mut body).expect("body");
    Recorded { path, body }
}

fn body_contains(body: &[u8], needle: &[u8]) -> bool {
    body.windows(needle.len()).any(|window| window == needle)
}

fn text(body: &[u8]) -> String {
    String::from_utf8_lossy(body).to_string()
}

fn single(id: i64) -> Reply {
    Reply {
        body: json!({"ok": true, "result": {"message_id": id}}),
    }
}

fn album(ids: &[i64]) -> Reply {
    let result: Vec<serde_json::Value> = ids.iter().map(|id| json!({"message_id": id})).collect();
    Reply {
        body: json!({"ok": true, "result": result}),
    }
}

fn file(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

fn client(base: &str) -> TelegramClient {
    TelegramClient::with_base_url("token", base).expect("client")
}

struct NoopSleeper;

impl hearth_telegram::Sleeper for NoopSleeper {
    fn sleep(&self, _duration: Duration) {}
}

fn sender(base: &str) -> TelegramSender {
    TelegramSender::new(client(base), Arc::new(NoopSleeper))
}

#[derive(Default)]
struct RecordingSleeper {
    sleeps: Mutex<Vec<Duration>>,
}

impl hearth_telegram::Sleeper for RecordingSleeper {
    fn sleep(&self, duration: Duration) {
        self.sleeps.lock().expect("sleeps").push(duration);
    }
}

#[test]
fn one_image_is_sent_with_the_message_tool() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    file(workspace.path(), "image.png", b"IMAGEDATA");
    let (base, server) = bot_server(vec![single(301)]);
    let tool = MessageTool::new(
        sender(&base),
        42,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );

    let output = tool
        .call(&json!({"path": "image.png", "caption": "hello"}))
        .expect("send image");
    assert!(!output.is_error);
    assert!(output.content.contains("301"));

    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.ends_with("/sendPhoto"));
    let body = text(&requests[0].body);
    assert!(body.contains("name=\"photo\""));
    assert!(body.contains("hello"));
    assert!(body_contains(&requests[0].body, b"IMAGEDATA"));
}

#[test]
fn one_document_is_sent_without_a_caption() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    file(workspace.path(), "notes.txt", b"TEXTDATA");
    let (base, server) = bot_server(vec![single(302)]);
    let tool = MessageTool::new(
        sender(&base),
        7,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );

    tool.call(&json!({"path": "notes.txt"})).expect("send doc");
    let requests = server.join().expect("server");
    assert!(requests[0].path.ends_with("/sendDocument"));
    let body = text(&requests[0].body);
    assert!(!body.contains("name=\"caption\""));
    assert!(body_contains(&requests[0].body, b"TEXTDATA"));
}

#[test]
fn two_images_are_sent_as_one_album() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    file(workspace.path(), "one.png", b"ONE");
    file(workspace.path(), "two.png", b"TWO");
    let (base, server) = bot_server(vec![album(&[401, 402])]);
    let tool = MessageTool::new(
        sender(&base),
        9,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );

    let output = tool
        .call(&json!({"paths": ["one.png", "two.png"], "caption": "pair"}))
        .expect("send album");
    assert!(output.content.contains("401"));
    assert!(output.content.contains("402"));

    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.ends_with("/sendMediaGroup"));
    let body = text(&requests[0].body);
    assert!(body.contains("attach://file0"));
    assert!(body.contains("attach://file1"));
    assert!(body.contains("pair"));
    assert!(body_contains(&requests[0].body, b"ONE"));
    assert!(body_contains(&requests[0].body, b"TWO"));
}

#[test]
fn a_mixed_group_is_sent_individually() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    file(workspace.path(), "one.png", b"ONE");
    file(workspace.path(), "notes.txt", b"TEXT");
    let (base, server) = bot_server(vec![single(501), single(502)]);
    let tool = MessageTool::new(
        sender(&base),
        3,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );

    tool.call(&json!({"paths": ["one.png", "notes.txt"]}))
        .expect("send files");
    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 2);
    assert!(requests[0].path.ends_with("/sendPhoto"));
    assert!(requests[1].path.ends_with("/sendDocument"));
}

#[test]
fn speech_is_written_and_sent_as_a_voice_note() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    let audio = vec![0x4f, 0x67, 0x67, 0x53, 0x01, 0x02];
    let (speech_base, speech_server_handle) = speech_server(audio.clone());
    let speaker: Arc<dyn Speaker> = Arc::new(
        SpeechClient::new(&speech_base, "key", "voice", "eleven_v4", "opus_48000_64")
            .expect("speech client"),
    );
    let (base, server) = bot_server(vec![single(601)]);
    let tool = MessageTool::new(
        sender(&base),
        11,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        Some(speaker),
    );

    tool.call(&json!({"speech": "hello there"}))
        .expect("send voice");
    speech_server_handle.join().expect("speech server");
    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.ends_with("/sendVoice"));
    assert!(body_contains(&requests[0].body, b"OggS"));
    assert!(body_contains(&requests[0].body, &audio));

    let files: Vec<_> = std::fs::read_dir(outbound.path())
        .expect("outbound")
        .collect::<Result<Vec<_>, _>>()
        .expect("entries");
    assert_eq!(files.len(), 1, "one voice file should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(files[0].path())
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn speech_without_a_speaker_is_an_error() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    let (base, _server) = bot_server(vec![]);
    let tool = MessageTool::new(
        sender(&base),
        1,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );
    let error = tool
        .call(&json!({"speech": "hello"}))
        .expect_err("no speaker");
    assert!(error.message.contains("speech"));
}

#[test]
fn invalid_calls_are_rejected_before_any_send() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    file(workspace.path(), "one.png", b"ONE");
    let (base, _server) = bot_server(vec![]);
    let tool = MessageTool::new(
        sender(&base),
        1,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );

    assert!(tool.call(&json!({})).is_err());
    assert!(tool.call(&json!({"path": "../escape.png"})).is_err());
    let many: Vec<&str> = (0..11).map(|_| "one.png").collect();
    assert!(tool.call(&json!({"paths": many})).is_err());
    assert!(
        tool.call(&json!({"path": "one.png", "paths": ["one.png"]}))
            .is_err()
    );
}

#[test]
fn the_message_tool_carries_a_thread_id() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    file(workspace.path(), "image.png", b"IMAGEDATA");
    let (base, server) = bot_server(vec![single(401)]);
    let tool = MessageTool::new(
        sender(&base),
        42,
        Some(9),
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );

    tool.call(&json!({"path": "image.png"}))
        .expect("send image");

    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.ends_with("/sendPhoto"));
    let body = text(&requests[0].body);
    assert!(body.contains("name=\"message_thread_id\""));
    assert!(body.contains("9"));
}

#[test]
fn message_tool_sends_wait_for_the_sender_limiter() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    file(workspace.path(), "one.png", b"ONE");
    file(workspace.path(), "two.png", b"TWO");
    let (base, server) = bot_server(vec![single(401), single(402)]);
    let sleeper = Arc::new(RecordingSleeper::default());
    let tool = MessageTool::new(
        TelegramSender::new(client(&base), sleeper.clone()),
        42,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );

    tool.call(&json!({"path": "one.png"})).expect("first send");
    tool.call(&json!({"path": "two.png"})).expect("second send");
    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 2);

    let sleeps = sleeper.sleeps.lock().expect("sleeps");
    assert!(
        sleeps
            .iter()
            .any(|duration| *duration >= Duration::from_millis(33)),
        "message sends did not share the sender limiter: {sleeps:?}"
    );
}

#[test]
fn a_configured_speaker_teaches_elevenlabs_tags() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    let speaker = SpeechClient::new(
        "http://127.0.0.1:1",
        "key",
        "voice",
        "eleven_v4",
        "opus_48000_64",
    )
    .expect("speech client");
    let tool = MessageTool::new(
        sender("http://127.0.0.1:1"),
        42,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        Some(Arc::new(speaker)),
    );
    let spec = tool.spec();
    assert!(spec.description.contains("Eleven"), "{}", spec.description);
    assert!(spec.description.contains("[warm"), "{}", spec.description);
    let speech = spec.parameters["properties"]["speech"]["description"]
        .as_str()
        .expect("speech description");
    assert!(speech.contains("Eleven"), "{speech}");
    assert!(speech.contains("[warm"), "{speech}");
}

#[test]
fn a_message_tool_without_speech_does_not_mention_tags() {
    let workspace = TempDir::new().expect("workspace");
    let spill = TempDir::new().expect("spill");
    let outbound = TempDir::new().expect("outbound");
    let tool = MessageTool::new(
        sender("http://127.0.0.1:1"),
        42,
        None,
        workspace.path(),
        spill.path(),
        outbound.path(),
        None,
    );
    let spec = tool.spec();
    assert!(!spec.description.contains("[warm"), "{}", spec.description);
}
