use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

use hearth_model::{MediaError, Transcriber, TranscriptionClient};
use tempfile::TempDir;

struct Captured {
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

fn fake_groq(
    response: serde_json::Value,
) -> (String, mpsc::Receiver<Captured>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake groq");
    let address = listener.local_addr().expect("address");
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept groq");
        let captured = read_request(&mut stream);
        sender.send(captured).expect("send captured");
        let body = response.to_string();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).expect("head");
        stream.write_all(body.as_bytes()).expect("body");
        stream.flush().expect("flush");
    });
    (format!("http://{address}"), receiver, handle)
}

fn read_request(stream: &mut TcpStream) -> Captured {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("request line");
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut content_length = 0usize;
    let mut authorization = None;
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
        if header.to_ascii_lowercase().starts_with("authorization:") {
            authorization = header
                .split_once(':')
                .map(|(_, value)| value.trim().to_owned());
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).expect("body");
    Captured {
        path,
        authorization,
        body,
    }
}

#[test]
fn transcription_posts_multipart_and_returns_text() {
    let (base, captured, server) = fake_groq(serde_json::json!({"text": "hello world"}));
    let directory = TempDir::new().expect("temp");
    let audio = directory.path().join("voice.ogg");
    std::fs::write(&audio, b"audio-bytes").expect("write audio");

    let client = TranscriptionClient::new(base, "test-key", "whisper-test").expect("client");
    let text = client.transcribe(&audio).expect("transcribe");
    assert_eq!(text, "hello world");

    let request = captured.recv().expect("captured");
    assert_eq!(request.path, "/openai/v1/audio/transcriptions");
    assert_eq!(request.authorization.as_deref(), Some("Bearer test-key"));
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.contains("name=\"model\""), "body: {body}");
    assert!(body.contains("whisper-test"), "body: {body}");
    assert!(body.contains("name=\"file\""), "body: {body}");
    assert!(body.contains("audio-bytes"), "body: {body}");
    server.join().expect("server");
}

#[test]
fn an_empty_transcript_is_an_error() {
    let (base, captured, server) = fake_groq(serde_json::json!({"text": "  "}));
    let directory = TempDir::new().expect("temp");
    let audio = directory.path().join("voice.ogg");
    std::fs::write(&audio, b"audio").expect("write audio");
    let client = TranscriptionClient::new(base, "test-key", "whisper-test").expect("client");
    let error = client.transcribe(&audio).expect_err("empty transcript");
    assert!(matches!(error, MediaError::Empty(_)));
    let _ = captured.recv().expect("captured");
    server.join().expect("server");
}
