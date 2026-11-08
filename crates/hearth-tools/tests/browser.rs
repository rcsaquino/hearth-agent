use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use hearth_tools::{BrowserTool, Tool};
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    path: String,
    body: String,
}

struct Reply {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
        }
    }

    fn bytes(content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type,
            body,
        }
    }
}

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(reply: impl Fn(&Recorded) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake camofox");
        listener.set_nonblocking(true).expect("nonblocking");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let request = read_request(&mut stream);
                        thread_requests
                            .lock()
                            .expect("requests")
                            .push(request.clone());
                        let reply = reply(&request);
                        let header = format!(
                            "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            reply.status,
                            reply.content_type,
                            reply.body.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&reply.body);
                        let _ = stream.flush();
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
            requests,
            stop,
            handle: Some(handle),
        }
    }

    fn request(&self, method: &str, path_fragment: &str) -> Recorded {
        self.requests
            .lock()
            .expect("requests")
            .iter()
            .find(|request| request.method == method && request.path.contains(path_fragment))
            .cloned()
            .unwrap_or_else(|| panic!("missing {method} {path_fragment}"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Recorded {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("request line");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).expect("header");
        if header == "\r\n" || header == "\n" || header.is_empty() {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body).expect("body");
    }
    Recorded {
        method,
        path,
        body: String::from_utf8_lossy(&body).to_string(),
    }
}

fn normal_reply(request: &Recorded) -> Reply {
    if request.method == "POST" && request.path == "/tabs" {
        return Reply::json(json!({"tabId": "tab-1", "title": "Example"}));
    }
    if request.method == "GET" && request.path.contains("/tabs/tab-1/snapshot") {
        return Reply::json(json!({"snapshot": "[e1] Hello page"}));
    }
    if request.method == "POST"
        && (request.path.contains("/tabs/tab-1/click") || request.path.contains("/tabs/tab-1/type"))
    {
        return Reply::json(json!({"ok": true}));
    }
    if request.method == "GET" && request.path.contains("/tabs/tab-1/screenshot") {
        return Reply::bytes(
            "image/png",
            vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        );
    }
    Reply::json(json!({"error": "not found"}))
}

fn tool(base: &str, workspace: &Path) -> BrowserTool {
    BrowserTool::new(base, workspace, Duration::from_secs(2)).expect("browser tool")
}

#[test]
fn opens_reads_clicks_and_types_through_the_fake_service() {
    let server = Server::start(normal_reply);
    let home = TempDir::new().expect("temp");
    let browser = tool(&server.base, home.path());

    let open = browser
        .call(&json!({"action": "open", "url": "https://example.com"}))
        .expect("open");
    assert!(open.content.contains("tab-1"), "{}", open.content);
    assert!(open.content.contains("Example"), "{}", open.content);
    let posted = server.request("POST", "/tabs");
    assert!(posted.body.contains("example.com"), "{}", posted.body);

    let read = browser
        .call(&json!({"action": "read", "tab_id": "tab-1"}))
        .expect("read");
    assert!(read.content.contains("Hello page"), "{}", read.content);
    let snapshot = server.request("GET", "/snapshot");
    assert!(snapshot.path.contains("userId=hearth"), "{}", snapshot.path);

    let click = browser
        .call(&json!({"action": "click", "tab_id": "tab-1", "ref": "e1"}))
        .expect("click");
    assert!(click.content.contains("clicked"), "{}", click.content);
    assert!(server.request("POST", "/click").body.contains("e1"));

    let typed = browser
        .call(&json!({"action": "type", "tab_id": "tab-1", "ref": "e1", "text": "hello"}))
        .expect("type");
    assert!(typed.content.contains("typed"), "{}", typed.content);
    assert!(server.request("POST", "/type").body.contains("hello"));
}

#[test]
fn saves_a_screenshot_inside_the_workspace() {
    let server = Server::start(normal_reply);
    let home = TempDir::new().expect("temp");
    let browser = tool(&server.base, home.path());
    let shot = browser
        .call(&json!({"action": "screenshot", "tab_id": "tab-1", "path": "shots/page.png"}))
        .expect("screenshot");
    assert!(shot.content.contains("shots/page.png"), "{}", shot.content);
    let file = home.path().join("shots/page.png");
    assert!(file.is_file());
    let bytes = std::fs::read(&file).expect("read image");
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).expect("mode").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn reports_an_unavailable_service() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let home = TempDir::new().expect("temp");
    let browser = tool(&base, home.path());
    let error = browser
        .call(&json!({"action": "open", "url": "https://example.com"}))
        .expect_err("unavailable");
    assert_eq!(error.code, "browser_unavailable", "{}", error.message);
}

#[test]
fn times_out_a_slow_service() {
    let server = Server::start(|_| {
        thread::sleep(Duration::from_millis(300));
        Reply::json(json!({"tabId": "tab-1"}))
    });
    let home = TempDir::new().expect("temp");
    let browser =
        BrowserTool::new(&server.base, home.path(), Duration::from_millis(80)).expect("tool");
    let error = browser
        .call(&json!({"action": "open", "url": "https://example.com"}))
        .expect_err("timeout");
    assert_eq!(error.code, "browser_timeout", "{}", error.message);
}

#[test]
fn caps_a_large_snapshot() {
    let server = Server::start(|request| {
        if request.path.contains("/snapshot") {
            return Reply::json(json!({"snapshot": "x".repeat(250_000)}));
        }
        normal_reply(request)
    });
    let home = TempDir::new().expect("temp");
    let browser = tool(&server.base, home.path());
    let read = browser
        .call(&json!({"action": "read", "tab_id": "tab-1"}))
        .expect("read");
    assert!(read.content.len() <= 100_100, "{}", read.content.len());
    assert!(read.content.contains("truncated"), "{}", read.content);
}
