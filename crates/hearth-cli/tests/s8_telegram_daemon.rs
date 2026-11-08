use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
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
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&value).expect("settings json"),
    )
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
    typing: AtomicUsize,
    reactions: AtomicUsize,
    rich: Mutex<Vec<String>>,
    rich_sends: AtomicUsize,
    stop: AtomicBool,
}

fn write_response_status(stream: &mut TcpStream, status: u16, body: &Value) {
    let text = serde_json::to_string(body).expect("json");
    write!(
        stream,
        "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        text.len(),
        text
    )
    .expect("write response");
}

fn fake_telegram_script(
    updates: Vec<Value>,
    fail_first_rich: bool,
) -> (String, Arc<TelegramState>, thread::JoinHandle<()>) {
    fake_telegram_script_with_delays(updates, fail_first_rich, Vec::new(), None)
}

fn fake_telegram_script_with_delays(
    updates: Vec<Value>,
    fail_first_rich: bool,
    delays: Vec<Duration>,
    stop_gate: Option<Arc<AtomicBool>>,
) -> (String, Arc<TelegramState>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind telegram");
    listener
        .set_nonblocking(true)
        .expect("nonblocking telegram");
    let base = format!("http://{}", listener.local_addr().expect("telegram addr"));
    let state = Arc::new(TelegramState::default());
    let thread_state = Arc::clone(&state);
    let next_update = Arc::new(AtomicUsize::new(0));
    let handle = thread::spawn(move || {
        while !thread_state.stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let request = read_request(&mut stream);
                    let method = request.path.rsplit('/').next().unwrap_or_default();
                    let body = match method {
                        "getUpdates" => {
                            let index = next_update.fetch_add(1, Ordering::SeqCst);
                            if let Some(delay) = delays.get(index) {
                                thread::sleep(*delay);
                            }
                            if index == 1
                                && let Some(gate) = &stop_gate
                            {
                                let deadline = Instant::now() + Duration::from_secs(5);
                                while !gate.load(Ordering::SeqCst) && Instant::now() < deadline {
                                    thread::sleep(Duration::from_millis(5));
                                }
                            }
                            if let Some(update) = updates.get(index) {
                                json!({"ok": true, "result": [update.clone()]})
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
                            let send_index = thread_state.rich_sends.fetch_add(1, Ordering::SeqCst);
                            if fail_first_rich && send_index == 0 {
                                write_response_status(
                                    &mut stream,
                                    500,
                                    &json!({"ok": false, "description": "synthetic send failure"}),
                                );
                                continue;
                            }
                            let markdown = request.body["rich_message"]["markdown"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            thread_state.rich.lock().expect("rich").push(markdown);
                            json!({"ok": true, "result": {"message_id": 500 + send_index}})
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

fn fake_telegram() -> (String, Arc<TelegramState>, thread::JoinHandle<()>) {
    fake_telegram_script(
        vec![json!({
            "update_id": 1,
            "message": {
                "message_id": 10,
                "date": 1_730_000_000,
                "chat": {"id": 42, "type": "private"},
                "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                "text": "hello"
            }
        })],
        false,
    )
}

fn fake_provider_answers(answers: Vec<&str>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider");
    let base = format!("http://{}", listener.local_addr().expect("provider addr"));
    let owned: Vec<String> = answers.into_iter().map(str::to_owned).collect();
    let handle = thread::spawn(move || {
        for answer in owned {
            let (mut stream, _) = listener.accept().expect("provider accept");
            let _ = read_request(&mut stream);
            let delta = json!({"choices": [{"delta": {"content": answer}}]});
            let finish = json!({"choices": [{"delta": {}, "finish_reason": "stop"}]});
            let usage = json!({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}});
            let body = format!(
                "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                delta, finish, usage
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("provider response");
        }
    });
    (format!("{base}/v1"), handle)
}

fn fake_provider() -> (String, thread::JoinHandle<()>) {
    fake_provider_answers(vec!["Telegram answer"])
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
    assert!(answer.contains("Telegram answer"), "answer was {answer:?}");
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

fn wait_for_rich_count(state: &TelegramState, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let rich = state.rich.lock().expect("rich").clone();
        if rich.len() >= count {
            return rich;
        }
        if Instant::now() >= deadline {
            panic!(
                "the daemon sent {} rich messages, wanted {count}",
                rich.len()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_daemon_recovers_an_uncertain_telegram_answer() {
    use hearth_core::{EffectStatus, Harness, SubmissionStatus};

    let (provider_base, provider_thread) =
        fake_provider_answers(vec!["First answer", "Second answer"]);
    let (telegram_base, telegram_state, telegram_thread) = fake_telegram_script_with_delays(
        vec![
            json!({
                "update_id": 1,
                "message": {
                    "message_id": 10,
                    "date": 1_730_000_000,
                    "chat": {"id": 42, "type": "private"},
                    "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                    "text": "hello"
                }
            }),
            json!({
                "update_id": 2,
                "message": {
                    "message_id": 11,
                    "date": 1_730_000_001,
                    "chat": {"id": 42, "type": "private"},
                    "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                    "text": "again"
                }
            }),
        ],
        true,
        vec![Duration::ZERO, Duration::from_millis(500)],
        None,
    );
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path(), &provider_base);
    add_telegram_base(home.path(), &telegram_base);

    let daemon = DaemonGuard::new(start_daemon(home.path()));
    let rich = wait_for_rich_count(&telegram_state, 2);
    assert!(
        rich[0].contains("Recovered reply") && rich[0].contains("First answer"),
        "first rich message was {:?}",
        rich[0]
    );
    assert!(
        rich[1].contains("Second answer"),
        "second rich message was {:?}",
        rich[1]
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
    assert_eq!(history.len(), 4, "history was {history:?}");
    let users: Vec<&str> = history
        .iter()
        .filter(|entry| entry.kind == "user")
        .map(|entry| entry.content[0]["text"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(users, vec!["hello", "again"]);
    let assistants: Vec<&str> = history
        .iter()
        .filter(|entry| entry.kind == "assistant")
        .map(|entry| entry.content[0]["text"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(assistants, vec!["First answer", "Second answer"]);

    let first = session
        .submission_by_request_id("tg:42:10")
        .expect("lookup first")
        .expect("first submission");
    let second = session
        .submission_by_request_id("tg:42:11")
        .expect("lookup second")
        .expect("second submission");
    assert_eq!(first.status, SubmissionStatus::Done);
    assert_eq!(second.status, SubmissionStatus::Done);

    let first_answer = history
        .iter()
        .find(|entry| Some(&entry.id) == first.answer_entry_id.as_ref())
        .expect("first answer entry");
    assert_eq!(first_answer.content[0]["text"], "First answer");
    let second_answer = history
        .iter()
        .find(|entry| Some(&entry.id) == second.answer_entry_id.as_ref())
        .expect("second answer entry");
    assert_eq!(second_answer.content[0]["text"], "Second answer");

    let first_effect = session
        .effect_by_key("telegram.send", &format!("tg:{}", first.id))
        .expect("read first effect")
        .expect("first effect");
    assert_eq!(first_effect.status, EffectStatus::Done);
    assert_eq!(
        first_effect.result.expect("first result")["recovered"],
        true,
        "the first answer should be marked recovered"
    );
    let second_effect = session
        .effect_by_key("telegram.send", &format!("tg:{}", second.id))
        .expect("read second effect")
        .expect("second effect");
    assert_eq!(second_effect.status, EffectStatus::Done);
    assert!(
        session
            .effects_awaiting_delivery("telegram.send")
            .expect("awaiting")
            .is_empty()
    );
}

struct SlowProvider {
    base: String,
    started: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl SlowProvider {
    fn new(answer: &str, delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider");
        listener
            .set_nonblocking(true)
            .expect("nonblocking provider");
        let base = format!(
            "http://{}/v1",
            listener.local_addr().expect("provider addr")
        );
        let started = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_started = Arc::clone(&started);
        let thread_stop = Arc::clone(&stop);
        let answer = answer.to_owned();
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = read_request(&mut stream);
                        let delta = json!({"choices": [{"delta": {"content": answer}}]});
                        let finish = json!({"choices": [{"delta": {}, "finish_reason": "stop"}]});
                        let usage = json!({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}});
                        let first = format!("data: {}\n\n", delta);
                        let rest =
                            format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", finish, usage);
                        let total = first.len() + rest.len();
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                        )
                        .expect("provider headers");
                        stream
                            .write_all(first.as_bytes())
                            .expect("provider first chunk");
                        stream.flush().expect("provider flush");
                        thread_started.store(true, Ordering::SeqCst);
                        thread::sleep(delay);
                        stream.write_all(rest.as_bytes()).expect("provider rest");
                        stream.flush().expect("provider flush rest");
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base,
            started,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for SlowProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct FakeTelegramGuard {
    base: String,
    state: Arc<TelegramState>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for FakeTelegramGuard {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn wait_for_flag(flag: &AtomicBool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !flag.load(Ordering::SeqCst) {
        if Instant::now() >= deadline {
            panic!("{message}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_daemon_stops_an_active_run() {
    use hearth_core::{EffectStatus, Harness, SubmissionStatus};

    let provider = SlowProvider::new("Partial answer", Duration::from_secs(1));
    let stop_gate = Arc::new(AtomicBool::new(false));
    let (telegram_base, telegram_state, telegram_thread) = fake_telegram_script_with_delays(
        vec![
            json!({
                "update_id": 1,
                "message": {
                    "message_id": 10,
                    "date": 1_730_000_000,
                    "chat": {"id": 42, "type": "private"},
                    "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                    "text": "hello"
                }
            }),
            json!({
                "update_id": 2,
                "message": {
                    "message_id": 11,
                    "date": 1_730_000_001,
                    "chat": {"id": 42, "type": "private"},
                    "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                    "text": "stop"
                }
            }),
        ],
        false,
        Vec::new(),
        Some(Arc::clone(&stop_gate)),
    );
    let telegram = FakeTelegramGuard {
        base: telegram_base,
        state: telegram_state,
        handle: Some(telegram_thread),
    };
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path(), &provider.base);
    add_telegram_base(home.path(), &telegram.base);

    let daemon = DaemonGuard::new(start_daemon(home.path()));
    wait_for_flag(&provider.started, "the provider never started");
    stop_gate.store(true, Ordering::SeqCst);
    let rich = wait_for_rich_count(&telegram.state, 1);
    assert!(
        rich[0].contains("Stopped."),
        "the daemon did not stop the run: {rich:?}"
    );
    assert!(telegram.state.typing.load(Ordering::SeqCst) > 0);
    assert_eq!(telegram.state.reactions.load(Ordering::SeqCst), 1);

    daemon.stop();
    drop(telegram);
    drop(provider);

    let harness = Harness::open(home.path()).expect("reopen harness");
    let session = harness.session();
    let conversation = session
        .conversation_by_transport("telegram", "42:0")
        .expect("lookup")
        .expect("telegram conversation");
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 2, "history was {history:?}");
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[0].content[0]["text"], "hello");
    assert_eq!(history[1].kind, "notice");
    assert_eq!(history[1].content["text"], "Stopped.");
    assert!(
        !history.iter().any(|entry| entry.kind == "assistant"),
        "a cancelled run saved an assistant entry: {history:?}"
    );
    let submission = session
        .submission_by_request_id("tg:42:10")
        .expect("submission lookup")
        .expect("submission");
    assert_eq!(submission.status, SubmissionStatus::Cancelled);
    assert!(submission.answer_entry_id.is_none());
    assert!(
        session
            .submission_by_request_id("tg:42:11")
            .expect("stop submission lookup")
            .is_none(),
        "the stop message must not create a submission"
    );
    let usage = session
        .latest_usage()
        .expect("usage")
        .expect("usage row from the provider call");
    assert_eq!(usage.input_tokens, 10);
    let effect = session
        .effect_by_key("telegram.send", &format!("tg:{}", submission.id))
        .expect("read effect")
        .expect("delivery effect");
    assert_eq!(effect.status, EffectStatus::Done);
    assert!(
        session
            .effects_awaiting_delivery("telegram.send")
            .expect("awaiting")
            .is_empty()
    );
}

fn wait_for_socket(home: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if home.join("hearth.sock").exists() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("daemon socket did not appear");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn hearth_status_reads_a_running_daemon() {
    let (telegram_base, telegram_state, telegram_thread) = fake_telegram_script(vec![], false);
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path(), "http://127.0.0.1:1/v1");
    add_telegram_base(home.path(), &telegram_base);
    {
        use hearth_core::Harness;
        use hearth_memory::{MemoryKind, MemoryService, NewMemoryInput, SourceKind};
        let harness = Harness::open(home.path()).expect("open memory home");
        let memory = MemoryService::new(harness.session().clone());
        memory
            .save(NewMemoryInput {
                content: "Prefers short replies.".to_owned(),
                kind: MemoryKind::Preference,
                tags: vec!["preferences".to_owned()],
                importance: 90,
                source_kind: SourceKind::User,
                source_ref: None,
                supersedes: None,
                valid_from: None,
                valid_to: None,
            })
            .expect("seed memory");
        let _ = memory.standing().expect("refresh standing");
    }
    let daemon = DaemonGuard::new(start_daemon(home.path()));
    wait_for_socket(home.path());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.path().join("hearth.sock"))
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "socket mode was {mode:o}");
    }

    let output = hearth()
        .args(["status", "--json"])
        .arg("--home")
        .arg(home.path())
        .output()
        .expect("run status");
    assert!(
        output.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: Value = serde_json::from_slice(&output.stdout).expect("status json");
    assert_eq!(status["model"], "fake-model");
    assert!(status["uptime_seconds"].as_u64().is_some());
    assert_eq!(status["tasks_running"], 0);
    assert_eq!(status["memory_items"], 1);
    assert_eq!(status["standing_memory_version"], 1);

    daemon.stop();
    telegram_state.stop.store(true, Ordering::SeqCst);
    let _ = telegram_thread.join();
}

fn start_watch(home: &Path) -> (Child, mpsc::Receiver<String>) {
    let mut child = hearth()
        .args(["watch", "--json"])
        .arg("--home")
        .arg(home)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn watch");
    let stdout = child.stdout.take().expect("watch stdout");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    (child, receiver)
}

#[test]
fn hearth_watch_receives_live_run_events() {
    let (provider_base, provider_thread) = fake_provider_answers(vec!["Watch answer"]);
    let (telegram_base, telegram_state, telegram_thread) = fake_telegram_script_with_delays(
        vec![json!({
            "update_id": 1,
            "message": {
                "message_id": 10,
                "date": 1_730_000_000,
                "chat": {"id": 42, "type": "private"},
                "from": {"id": 42, "is_bot": false, "first_name": "Sam"},
                "text": "hello"
            }
        })],
        false,
        vec![Duration::from_secs(2)],
        None,
    );
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path(), &provider_base);
    add_telegram_base(home.path(), &telegram_base);
    let daemon = DaemonGuard::new(start_daemon(home.path()));
    wait_for_socket(home.path());

    let (mut watch, lines) = start_watch(home.path());
    let ack = lines
        .recv_timeout(Duration::from_secs(5))
        .expect("watch ack");
    assert!(ack.contains("\"ok\":true"), "ack was {ack}");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut kinds = Vec::new();
    while Instant::now() < deadline {
        let Ok(line) = lines.recv_timeout(Duration::from_secs(5)) else {
            break;
        };
        let value: Value = serde_json::from_str(&line).expect("watch event json");
        let kind = value["kind"].as_str().unwrap_or_default().to_owned();
        kinds.push(kind.clone());
        if kind == "run_settled" {
            break;
        }
    }
    assert!(
        kinds.iter().any(|kind| kind == "run_started"),
        "kinds: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "turn_started"),
        "kinds: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "text_delta"),
        "kinds: {kinds:?}"
    );
    assert!(kinds.iter().any(|kind| kind == "usage"), "kinds: {kinds:?}");
    assert!(
        kinds.iter().any(|kind| kind == "run_settled"),
        "kinds: {kinds:?}"
    );

    let _ = watch.kill();
    let _ = watch.wait();
    daemon.stop();
    telegram_state.stop.store(true, Ordering::SeqCst);
    let _ = telegram_thread.join();
    let _ = provider_thread.join();
}

fn seed_reminder(home: &Path, wake_at: i64, message: &str) -> (String, String) {
    use hearth_core::{Harness, NewTask};
    let harness = Harness::open(home).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "telegram", "42:0")
        .expect("conversation");
    let task = session
        .create_sleeping_task(
            NewTask::new(
                "reminder",
                1,
                "remind",
                json!({
                    "chat_id": 42,
                    "thread_id": null,
                    "message": message,
                    "repeat_seconds": null,
                    "first_wake": wake_at
                }),
            )
            .conversation(&conversation.id),
            wake_at,
        )
        .expect("reminder task");
    (task.id, conversation.id)
}

#[test]
fn a_reminder_fires_once_after_recovery() {
    use hearth_core::{EffectStatus, Harness, TaskStatus};
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path(), "http://127.0.0.1:9");
    let (telegram_base, telegram_state, telegram_thread) = fake_telegram_script(vec![], false);
    add_telegram_base(home.path(), &telegram_base);
    let now = {
        let harness = Harness::open(home.path()).expect("harness");
        harness.session().now_micros()
    };
    let (task_id, _conversation_id) = seed_reminder(home.path(), now - 1, "Reminder: drink water");

    let daemon = DaemonGuard::new(start_daemon(home.path()));
    let rich = wait_for_rich_count(&telegram_state, 1);
    assert!(rich[0].contains("Reminder: drink water"), "{rich:?}");
    daemon.stop();
    telegram_state.stop.store(true, Ordering::SeqCst);
    let _ = telegram_thread.join();

    let harness = Harness::open(home.path()).expect("reopen");
    let task = harness
        .session()
        .task(&task_id)
        .expect("task read")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Done);
    let effect = harness
        .session()
        .effect_by_key(
            "telegram.send",
            &format!("reminder:{}:{}", task_id, now - 1),
        )
        .expect("effect read")
        .expect("effect exists");
    assert_eq!(effect.status, EffectStatus::Done);
}

#[test]
fn a_reminder_survives_a_restart() {
    use hearth_core::{Harness, TaskStatus};
    let home = tempfile::TempDir::new().expect("temp home");
    init_home(home.path(), "http://127.0.0.1:9");
    let (telegram_base, telegram_state, telegram_thread) = fake_telegram_script(vec![], false);
    add_telegram_base(home.path(), &telegram_base);
    let now = {
        let harness = Harness::open(home.path()).expect("harness");
        harness.session().now_micros()
    };
    let wake = now + 2_000_000;
    let (task_id, _conversation_id) = seed_reminder(home.path(), wake, "Restart reminder");

    let daemon = DaemonGuard::new(start_daemon(home.path()));
    thread::sleep(Duration::from_millis(300));
    daemon.stop();
    thread::sleep(Duration::from_millis(2_200));

    let daemon = DaemonGuard::new(start_daemon(home.path()));
    let rich = wait_for_rich_count(&telegram_state, 1);
    assert!(rich[0].contains("Restart reminder"), "{rich:?}");
    daemon.stop();
    telegram_state.stop.store(true, Ordering::SeqCst);
    let _ = telegram_thread.join();

    let harness = Harness::open(home.path()).expect("reopen");
    let task = harness
        .session()
        .task(&task_id)
        .expect("task read")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Done);
}
