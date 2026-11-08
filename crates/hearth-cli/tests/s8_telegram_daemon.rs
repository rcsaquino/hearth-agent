use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn hearth() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hearth"))
}

fn init_home(home: &Path, provider_base: &str) {
    let output = hearth()
        .args(["init", "--non-interactive"])
        .arg("--home")
        .arg(home)
        .args(["--telegram-token", "123:fake-token"])
        .args(["--allowed-user-id", "42"])
        .args(["--base-url", provider_base])
        .args(["--api-key", "fake-key"])
        .args(["--model", "fake-model"])
        .args(["--timezone", "UTC"])
        .output()
        .expect("run init");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn add_telegram_base(home: &Path, base: &str) {
    let path = home.join("settings.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read settings"))
            .expect("parse settings");
    value["telegram"]["base_url"] = json!(base);
    std::fs::write(&path, serde_json::to_string_pretty(&value).expect("settings json"))
        .expect("write settings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("settings mode");
    }
}

struct Request {
    path: String,
    body: Value,
}

fn read_request(stream: &mut TcpStream) -> Request {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("request line");
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
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
    Request {
        path,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    }
}

fn write_response(stream: &mut TcpStream, body: &Value) {
    let text = serde_json::to_string(body).expect("json");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        text.len(),
        text
    )
    .expect("write response");
}

#[derive(Default)]
struct TelegramState {
    served: AtomicBool,
    typing: AtomicUsize,
    reactions: AtomicUsize,
    rich: Mutex<Vec<String>>,
    stop: AtomicBool,
}

fn fake_telegram() -> (String, Arc<TelegramState>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind telegram");
    listener
        .set_nonblocking(true)
        .expect("nonblocking telegram");
    let base = format!("http://{}", listener.local_addr().expect("telegram addr"));
    let state = Arc::new(TelegramState::default());
    let thread_state = Arc::clone(&state);
    let handle = thread::spawn(move || {
        while !thread_state.stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let request = read_request(&mut stream);
                    let method = request.path.rsplit('/').next().unwrap_or_default();
                    let body = match method {
                        "getUpdates" => {
                            if !thread_state.served.swap(true, Ordering::SeqCst) {
                                json!({"ok": true, "result": [{
                                    "update_id": 1,
                                    "message": {
                                        "message_id": 10,
                                        "date": 1_730_000_000,
                                        "chat": {"id": 42, "type": "private"},
                                        "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                                        "text": "hello"
                                    }
                                }]})
                            } else {
                                thread::sleep(Duration::from_millis(20));
                                json!({"ok": true, "result": []})
                            }
                        }
                        "setMessageReaction" => {
                            thread_state.reactions.fetch_add(1, Ordering::SeqCst);
                            json!({"ok": true, "result": true})
                        }
                        "sendChatAction" => {
                            thread_state.typing.fetch_add(1, Ordering::SeqCst);
                            json!({"ok": true, "result": true})
                        }
                        "sendRichMessage" => {
                            let markdown = request.body["rich_message"]["markdown"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            thread_state.rich.lock().expect("rich").push(markdown);
                            json!({"ok": true, "result": {"message_id": 500}})
                        }
                        _ => json!({"ok": true, "result": true}),
                    };
                    write_response(&mut stream, &body);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    (base, state, handle)
}

fn fake_provider() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider");
    let base = format!("http://{}", listener.local_addr().expect("provider addr"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("provider accept");
        let _ = read_request(&mut stream);
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Telegram answer\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3,\"total_tokens\":13}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("provider response");
    });
    (format!("{base}/v1"), handle)
}

struct DaemonGuard(Option<Child>);

impl DaemonGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn stop(mut self) {
        if let Some(child) = self.0.take() {
            stop_daemon(child);
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn start_daemon(home: &Path) -> Child {
    hearth()
        .args(["daemon"])
        .arg("--home")
        .arg(home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon")
}

fn stop_daemon(mut child: Child) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait().expect("poll daemon") {
            Some(_) => return,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            None => {
                let _ = child.kill();
                panic!("daemon did not stop on SIGTERM");
            }
        }
    }
}

fn wait_for_answer(state: &TelegramState) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(answer) = state.rich.lock().expect("rich").first().cloned() {
            return answer;
        }
        if Instant::now() >= deadline {
            panic!("the daemon did not send a telegram answer");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_daemon_answers_a_telegram_message() {
    use hearth_core::{Harness, SubmissionStatus};

    let (provider_base, provider_thread) = fake_provider();
    let (telegram_base, telegram_state, telegram_thread) = fake_telegram();
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path(), &provider_base);
    add_telegram_base(home.path(), &telegram_base);

    let daemon = DaemonGuard::new(start_daemon(home.path()));
    let answer = wait_for_answer(&telegram_state);
    assert!(
        answer.contains("Telegram answer"),
        "answer was {answer:?}"
    );
    assert!(telegram_state.typing.load(Ordering::SeqCst) > 0);
    assert!(telegram_state.reactions.load(Ordering::SeqCst) > 0);

    daemon.stop();
    telegram_state.stop.store(true, Ordering::SeqCst);
    let _ = telegram_thread.join();
    let _ = provider_thread.join();

    let harness = Harness::open(home.path()).expect("reopen harness");
    let session = harness.session();
    let conversation = session
        .conversation_by_transport("telegram", "42:0")
        .expect("lookup")
        .expect("telegram conversation");
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[0].content[0]["text"], "hello");
    assert_eq!(history[1].kind, "assistant");
    assert_eq!(history[1].content[0]["text"], "Telegram answer");

    let submission = session
        .submission_by_request_id("tg:42:10")
        .expect("submission lookup")
        .expect("submission");
    assert_eq!(submission.status, SubmissionStatus::Done);
    assert!(session.latest_usage().expect("usage").is_some());
}
