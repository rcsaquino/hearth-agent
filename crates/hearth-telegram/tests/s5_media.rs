use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};

use hearth_telegram::{TelegramClient, TelegramError};

struct FakeFileApi {
    address: std::net::SocketAddr,
    handle: JoinHandle<()>,
}

impl FakeFileApi {
    fn start(file_path: &'static str, bytes: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake file api");
        let address = listener.local_addr().expect("address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("getFile connection");
            let (method, path) = read_request(&mut stream);
            assert_eq!(method, "POST");
            assert!(path.ends_with("/getFile"), "path: {path}");
            write_json(
                &mut stream,
                &serde_json::json!({
                    "ok": true,
                    "result": {
                        "file_id": "file-1",
                        "file_size": bytes.len(),
                        "file_path": file_path,
                    }
                }),
            );

            let (mut stream, _) = listener.accept().expect("download connection");
            let (method, path) = read_request(&mut stream);
            assert_eq!(method, "GET");
            assert!(
                path.contains("/file/bot123:fake/photos/a%20b.jpg"),
                "encoded file path: {path}"
            );
            write_bytes(&mut stream, &bytes, "application/octet-stream");
        });
        Self { address, handle }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn finish(self) {
        self.handle.join().expect("fake file api thread");
    }
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header line");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if line.to_ascii_lowercase().starts_with("content-length:") {
            content_length = line
                .split(':')
                .nth(1)
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).expect("request body");
    }
    (method, path)
}

fn write_json(stream: &mut TcpStream, value: &serde_json::Value) {
    write_bytes(stream, value.to_string().as_bytes(), "application/json");
}

fn write_bytes(stream: &mut TcpStream, body: &[u8], content_type: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("response head");
    stream.write_all(body).expect("response body");
    stream.flush().expect("response flush");
}

fn client(api: &FakeFileApi) -> TelegramClient {
    TelegramClient::with_base_url("123:fake", api.base_url()).expect("client")
}

#[test]
fn get_file_and_download_follow_the_bot_api_paths() {
    let api = FakeFileApi::start("photos/a b.jpg", vec![1, 2, 3]);
    let client = client(&api);

    let info = client.get_file("file-1").expect("get file");
    assert_eq!(info.file_id, "file-1");
    assert_eq!(info.file_size, Some(3));
    assert_eq!(info.file_path.as_deref(), Some("photos/a b.jpg"));

    let bytes = client
        .download_file("photos/a b.jpg", 10)
        .expect("download");
    assert_eq!(bytes, vec![1, 2, 3]);
    api.finish();
}

#[test]
fn a_download_over_the_cap_is_refused() {
    let api = FakeFileApi::start("photos/a b.jpg", vec![1, 2, 3, 4, 5]);
    let client = client(&api);
    client.get_file("file-1").expect("get file");
    let error = client
        .download_file("photos/a b.jpg", 4)
        .expect_err("over the cap");
    assert!(matches!(error, TelegramError::TooLarge { limit: 4 }));
    api.finish();
}
