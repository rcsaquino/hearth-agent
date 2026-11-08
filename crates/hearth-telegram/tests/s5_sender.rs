use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use hearth_telegram::{SendOutcome, Sleeper, TelegramClient, TelegramSender};
use serde_json::Value;

struct Reply {
    status: u16,
    body: Value,
}

struct Recorded {
    method: String,
    body: Value,
}

struct Server {
    base: String,
    handle: thread::JoinHandle<Vec<Recorded>>,
}

impl Server {
    fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake bot api");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().expect("accept");
                let request = read_request(&mut stream);
                requests.push(request);
                let body = serde_json::to_string(&reply.body).expect("json");
                let reason = match reply.status {
                    200 => "OK",
                    400 => "Bad Request",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    _ => "Error",
                };
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.status,
                    reason,
                    body.len(),
                    body
                )
                .expect("write response");
            }
            requests
        });
        Self { base, handle }
    }

    fn finish(self) -> Vec<Recorded> {
        self.handle.join().expect("server thread")
    }
}

fn ok() -> Reply {
    Reply {
        status: 200,
        body: serde_json::json!({"ok": true, "result": true}),
    }
}

fn error(status: u16, code: i64, description: &str, retry_after: Option<u64>) -> Reply {
    let mut body = serde_json::json!({
        "ok": false,
        "error_code": code,
        "description": description,
    });
    if let Some(seconds) = retry_after {
        body["parameters"] = serde_json::json!({"retry_after": seconds});
    }
    Reply { status, body }
}

fn read_request(stream: &mut std::net::TcpStream) -> Recorded {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    while !buffer.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).expect("read header") == 0 {
            break;
        }
        buffer.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&buffer);
    let length = text
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).expect("read body");
    let method = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| path.rsplit('/').next())
        .unwrap_or("unknown")
        .to_owned();
    Recorded {
        method,
        body: serde_json::from_slice(&body).expect("json body"),
    }
}

#[derive(Default)]
struct RecordingSleeper {
    sleeps: Mutex<Vec<Duration>>,
}

impl Sleeper for RecordingSleeper {
    fn sleep(&self, duration: Duration) {
        self.sleeps.lock().expect("sleeper").push(duration);
    }
}

fn sender(base: &str) -> (TelegramSender, Arc<RecordingSleeper>) {
    let client = TelegramClient::with_base_url("TEST", base).expect("client");
    let sleeper = Arc::new(RecordingSleeper::default());
    (TelegramSender::new(client, sleeper.clone()), sleeper)
}

#[test]
fn sends_a_rich_reply() {
    let server = Server::start(vec![ok()]);
    let (sender, _) = sender(&server.base);
    let outcome = sender.send_reply(42, "**hello**").expect("send");
    assert_eq!(outcome, SendOutcome::Rich);
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "sendRichMessage");
    assert_eq!(requests[0].body["chat_id"], 42);
    assert_eq!(requests[0].body["rich_message"]["markdown"], "**hello**");
}

#[test]
fn falls_back_to_entities_when_the_rich_message_is_rejected() {
    let server = Server::start(vec![error(400, 400, "bad rich message", None), ok()]);
    let (sender, _) = sender(&server.base);
    let outcome = sender
        .send_reply(42, "Hello **bold** and [link](https://example.com).")
        .expect("send");
    assert_eq!(outcome, SendOutcome::Fallback { parts: 1 });
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "sendRichMessage");
    assert_eq!(requests[1].method, "sendMessage");
    assert_eq!(requests[1].body["text"], "Hello bold and link.");
    let entities = requests[1].body["entities"].as_array().expect("entities");
    let bold = entities
        .iter()
        .find(|entity| entity["type"] == "bold")
        .expect("bold entity");
    assert_eq!(bold["offset"], 6);
    assert_eq!(bold["length"], 4);
    let link = entities
        .iter()
        .find(|entity| entity["type"] == "text_link")
        .expect("link entity");
    assert_eq!(link["offset"], 15);
    assert_eq!(link["length"], 4);
    assert_eq!(link["url"], "https://example.com");
}

#[test]
fn splits_a_long_rejected_reply_into_marked_parts() {
    let markdown = format!("```rust\n{}\n```", "a".repeat(10_000));
    let server = Server::start(vec![
        error(400, 400, "too long for rich", None),
        ok(),
        ok(),
        ok(),
    ]);
    let (sender, _) = sender(&server.base);
    let outcome = sender.send_reply(42, &markdown).expect("send");
    assert_eq!(outcome, SendOutcome::Fallback { parts: 3 });
    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, "sendRichMessage");
    let mut total_units = 0;
    for (index, request) in requests.iter().skip(1).enumerate() {
        assert_eq!(request.method, "sendMessage");
        let text = request.body["text"].as_str().expect("text");
        assert!(text.encode_utf16().count() <= 4096, "part too long");
        assert!(text.ends_with(&format!("({}/{})", index + 1, 3)), "missing marker: {text:?}");
        total_units += text.encode_utf16().count();
        let has_pre = request.body["entities"]
            .as_array()
            .is_some_and(|entities| entities.iter().any(|entity| entity["type"] == "pre"));
        assert!(has_pre, "part lost its pre entity");
    }
    assert!(total_units >= 10_000);
}

#[test]
fn retries_a_429_with_the_requested_wait() {
    let server = Server::start(vec![error(429, 429, "slow down", Some(2)), ok()]);
    let (sender, sleeper) = sender(&server.base);
    let outcome = sender.send_reply(42, "hi").expect("send");
    assert_eq!(outcome, SendOutcome::Rich);
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.method == "sendRichMessage"));
    let sleeps = sleeper.sleeps.lock().expect("sleeps");
    assert!(
        sleeps.contains(&Duration::from_secs(2)),
        "missing 2 second wait: {:?}",
        *sleeps
    );
}
