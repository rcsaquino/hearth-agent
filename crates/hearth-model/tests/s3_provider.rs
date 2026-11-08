use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use hearth_model::{
    ChatMessage, ChatRequest, ContentBlock, OpenAiCompatibleProvider, Provider, Role, Sleeper,
    StreamEvent, ToolSpec,
};
use serde_json::{Value, json};

struct TestResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: String,
}

impl TestResponse {
    #[allow(dead_code)]
    fn sse(chunks: &[&str]) -> Self {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(chunk);
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");
        Self {
            status: 200,
            reason: "OK",
            content_type: "text/event-stream",
            body,
        }
    }

    #[allow(dead_code)]
    fn text(status: u16, reason: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain",
            body: body.to_owned(),
        }
    }

    #[allow(dead_code)]
    fn raw(status: u16, reason: &'static str, content_type: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            content_type,
            body: body.to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

struct FakeServer {
    address: SocketAddr,
    requests: mpsc::Receiver<CapturedRequest>,
    handle: JoinHandle<()>,
}

impl FakeServer {
    fn start(responses: Vec<TestResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let (sender, requests) = mpsc::channel();
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                let request = read_request(&mut stream);
                sender.send(request).expect("send captured request");
                write_response(&mut stream, &response);
            }
        });
        Self {
            address,
            requests,
            handle,
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn request(&self) -> CapturedRequest {
        self.requests
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request")
    }

    fn finish(self) {
        self.handle.join().expect("test server thread");
    }
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header line");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).expect("request body");
    CapturedRequest {
        method,
        path,
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    }
}

fn write_response(stream: &mut TcpStream, response: &TestResponse) {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    );
    stream.write_all(head.as_bytes()).expect("response head");
    stream
        .write_all(response.body.as_bytes())
        .expect("response body");
    stream.flush().expect("response flush");
}

#[derive(Default)]
struct NoopSleeper {
    sleeps: Mutex<Vec<Duration>>,
}

impl Sleeper for NoopSleeper {
    fn sleep(&self, duration: Duration) {
        self.sleeps.lock().expect("sleep lock").push(duration);
    }
}

impl NoopSleeper {
    #[allow(dead_code)]
    fn sleeps(&self) -> usize {
        self.sleeps.lock().expect("sleep lock").len()
    }
}

fn provider(server: &FakeServer, sleeper: Arc<dyn Sleeper>) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::with_sleeper(
        server.base_url(),
        "test-key",
        vec![("x-extra".to_owned(), "yes".to_owned())],
        Some("x-session".to_owned()),
        sleeper,
    )
    .expect("provider")
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "test-model".to_owned(),
        messages: vec![
            ChatMessage {
                role: Role::System,
                blocks: vec![ContentBlock::Text("sys".to_owned())],
            },
            ChatMessage {
                role: Role::User,
                blocks: vec![ContentBlock::Text("hi".to_owned())],
            },
        ],
        tools: vec![ToolSpec {
            name: "files".to_owned(),
            description: "read and write".to_owned(),
            parameters: json!({"type": "object"}),
        }],
        temperature: Some(0.2),
        max_tokens: Some(100),
        session_id: Some("conv-1".to_owned()),
    }
}

#[test]
fn a_text_stream_sends_the_request_and_yields_deltas() {
    let server = FakeServer::start(vec![TestResponse::sse(&[
        r#"{"choices":[{"delta":{"content":"Hel"},"finish_reason":null}]}"#,
        r#"{"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
    ])]);
    let sleeper = Arc::new(NoopSleeper::default());
    let provider = provider(&server, sleeper);
    let events: Vec<StreamEvent> = provider
        .stream(request())
        .expect("stream")
        .collect::<Result<_, _>>()
        .expect("events");
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("Hel".to_owned()),
            StreamEvent::TextDelta("lo".to_owned()),
            StreamEvent::Done {
                finish_reason: Some("stop".to_owned())
            },
        ]
    );

    let captured = server.request();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/chat/completions");
    assert_eq!(
        captured.headers.get("authorization").map(String::as_str),
        Some("Bearer test-key")
    );
    assert_eq!(
        captured.headers.get("x-extra").map(String::as_str),
        Some("yes")
    );
    assert_eq!(
        captured.headers.get("x-session").map(String::as_str),
        Some("conv-1")
    );
    assert_eq!(captured.body["model"], "test-model");
    assert_eq!(captured.body["stream"], true);
    assert_eq!(captured.body["stream_options"]["include_usage"], true);
    assert_eq!(captured.body["max_tokens"], 100);
    assert!((captured.body["temperature"].as_f64().expect("temperature") - 0.2).abs() < 1e-6);
    assert_eq!(captured.body["tools"][0]["type"], "function");
    assert_eq!(captured.body["tools"][0]["function"]["name"], "files");
    assert_eq!(captured.body["messages"][0]["role"], "system");
    assert_eq!(captured.body["messages"][0]["content"], "sys");
    assert_eq!(captured.body["messages"][1]["role"], "user");
    assert_eq!(captured.body["messages"][1]["content"], "hi");
    server.finish();
}

#[test]
fn tool_call_deltas_assemble_into_one_call() {
    let server = FakeServer::start(vec![TestResponse::sse(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"files","arguments":"{\"pa"}}]},"finish_reason":null}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.txt\"}"}}]},"finish_reason":"tool_calls"}]}"#,
    ])]);
    let provider = provider(&server, Arc::new(NoopSleeper::default()));
    let events: Vec<StreamEvent> = provider
        .stream(request())
        .expect("stream")
        .collect::<Result<_, _>>()
        .expect("events");
    assert_eq!(
        events,
        vec![
            StreamEvent::ToolCall(hearth_model::ToolCall {
                id: "call_1".to_owned(),
                name: "files".to_owned(),
                arguments: json!({"path": "a.txt"}),
            }),
            StreamEvent::Done {
                finish_reason: Some("tool_calls".to_owned())
            },
        ]
    );
    server.request();
    server.finish();
}

#[test]
fn usage_chunk_maps_every_token_field() {
    let server = FakeServer::start(vec![TestResponse::sse(&[
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120,"prompt_tokens_details":{"cached_tokens":40},"cache_creation_input_tokens":10,"completion_tokens_details":{"reasoning_tokens":5}}}"#,
    ])]);
    let provider = provider(&server, Arc::new(NoopSleeper::default()));
    let events: Vec<StreamEvent> = provider
        .stream(request())
        .expect("stream")
        .collect::<Result<_, _>>()
        .expect("events");
    assert_eq!(
        events,
        vec![
            StreamEvent::Usage(hearth_model::Usage {
                input_tokens: 100,
                cache_read_tokens: 40,
                cache_write_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 5,
            }),
            StreamEvent::Done {
                finish_reason: Some("stop".to_owned())
            },
        ]
    );
    server.request();
    server.finish();
}

#[test]
fn reasoning_deltas_are_separate_from_text() {
    let server = FakeServer::start(vec![TestResponse::sse(&[
        r#"{"choices":[{"delta":{"reasoning_content":"think","content":"answer"},"finish_reason":"stop"}]}"#,
    ])]);
    let provider = provider(&server, Arc::new(NoopSleeper::default()));
    let events: Vec<StreamEvent> = provider
        .stream(request())
        .expect("stream")
        .collect::<Result<_, _>>()
        .expect("events");
    assert_eq!(
        events,
        vec![
            StreamEvent::ThinkingDelta("think".to_owned()),
            StreamEvent::TextDelta("answer".to_owned()),
            StreamEvent::Done {
                finish_reason: Some("stop".to_owned())
            },
        ]
    );
    server.request();
    server.finish();
}

#[test]
fn retryable_errors_retry_until_success() {
    let server = FakeServer::start(vec![
        TestResponse::text(503, "Service Unavailable", "busy"),
        TestResponse::text(503, "Service Unavailable", "busy"),
        TestResponse::sse(&[r#"{"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#]),
    ]);
    let sleeper = Arc::new(NoopSleeper::default());
    let provider = provider(&server, sleeper.clone());
    let events: Vec<StreamEvent> = provider
        .stream(request())
        .expect("stream")
        .collect::<Result<_, _>>()
        .expect("events");
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("ok".to_owned()),
            StreamEvent::Done {
                finish_reason: Some("stop".to_owned())
            },
        ]
    );
    assert_eq!(sleeper.sleeps(), 2);
    assert_eq!(server.request().path, "/chat/completions");
    assert_eq!(server.request().path, "/chat/completions");
    assert_eq!(server.request().path, "/chat/completions");
    server.finish();
}

#[test]
fn client_errors_do_not_retry() {
    let server = FakeServer::start(vec![TestResponse::text(401, "Unauthorized", "no")]);
    let sleeper = Arc::new(NoopSleeper::default());
    let provider = provider(&server, sleeper.clone());
    let error = match provider.stream(request()) {
        Ok(_) => panic!("401 should fail"),
        Err(error) => error,
    };
    assert_eq!(error.status(), Some(401));
    assert!(!error.retryable());
    assert_eq!(sleeper.sleeps(), 0);
    server.request();
    server.finish();
}

#[test]
fn malformed_sse_is_a_protocol_error() {
    let server = FakeServer::start(vec![TestResponse::raw(
        200,
        "OK",
        "text/event-stream",
        "data: not-json\n\n",
    )]);
    let provider = provider(&server, Arc::new(NoopSleeper::default()));
    let mut stream = provider.stream(request()).expect("stream");
    let error = match stream.next() {
        Some(Err(error)) => error,
        other => panic!("expected a protocol error, got {other:?}"),
    };
    assert!(matches!(error, hearth_model::ProviderError::Protocol(_)));
    assert!(stream.next().is_none());
    server.request();
    server.finish();
}
