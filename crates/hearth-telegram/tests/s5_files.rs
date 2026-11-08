use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;

use hearth_telegram::{MediaKind, TelegramClient};
use tempfile::TempDir;

struct Reply {
    body: serde_json::Value,
}

struct Recorded {
    path: String,
    body: Vec<u8>,
}

fn server(replies: Vec<Reply>) -> (String, thread::JoinHandle<Vec<Recorded>>) {
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

fn single(message_id: i64) -> Reply {
    Reply {
        body: serde_json::json!({"ok": true, "result": {"message_id": message_id}}),
    }
}

fn album(ids: &[i64]) -> Reply {
    let result: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| serde_json::json!({"message_id": id}))
        .collect();
    Reply {
        body: serde_json::json!({"ok": true, "result": result}),
    }
}

fn file(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

#[test]
fn a_single_photo_is_sent_as_multipart_with_a_caption() {
    let directory = TempDir::new().expect("temp");
    let photo = file(directory.path(), "photo.png", b"PNGDATA");
    let (base, server) = server(vec![single(101)]);
    let client = TelegramClient::with_base_url("token", &base).expect("client");

    let id = client
        .send_photo(42, &photo, Some("hello"))
        .expect("send photo");
    assert_eq!(id, 101);

    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.ends_with("/sendPhoto"));
    let body = text(&requests[0].body);
    assert!(body.contains("name=\"chat_id\""));
    assert!(body.contains("42"));
    assert!(body.contains("name=\"photo\""));
    assert!(body.contains("photo.png"));
    assert!(body.contains("name=\"caption\""));
    assert!(body.contains("hello"));
    assert!(body_contains(&requests[0].body, b"PNGDATA"));
}

#[test]
fn a_document_omits_an_absent_caption() {
    let directory = TempDir::new().expect("temp");
    let document = file(directory.path(), "notes.txt", b"TEXTDATA");
    let (base, server) = server(vec![single(102)]);
    let client = TelegramClient::with_base_url("token", &base).expect("client");

    let id = client
        .send_document(7, &document, None)
        .expect("send document");
    assert_eq!(id, 102);

    let requests = server.join().expect("server");
    assert!(requests[0].path.ends_with("/sendDocument"));
    let body = text(&requests[0].body);
    assert!(body.contains("name=\"document\""));
    assert!(!body.contains("name=\"caption\""));
    assert!(body_contains(&requests[0].body, b"TEXTDATA"));
}

#[test]
fn two_images_are_sent_as_one_album() {
    let directory = TempDir::new().expect("temp");
    let first = file(directory.path(), "one.png", b"FIRST");
    let second = file(directory.path(), "two.png", b"SECOND");
    let (base, server) = server(vec![album(&[201, 202])]);
    let client = TelegramClient::with_base_url("token", &base).expect("client");

    let ids = client
        .send_media_group(
            9,
            &[
                (MediaKind::Photo, first),
                (MediaKind::Photo, second),
            ],
            Some("album"),
        )
        .expect("send album");
    assert_eq!(ids, vec![201, 202]);

    let requests = server.join().expect("server");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.ends_with("/sendMediaGroup"));
    let body = text(&requests[0].body);
    assert!(body.contains("name=\"media\""));
    assert!(body.contains("attach://file0"));
    assert!(body.contains("attach://file1"));
    assert!(body.contains("name=\"file0\""));
    assert!(body.contains("name=\"file1\""));
    assert!(body.contains("album"));
    assert!(body_contains(&requests[0].body, b"FIRST"));
    assert!(body_contains(&requests[0].body, b"SECOND"));
}
