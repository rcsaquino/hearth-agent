use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

use hearth_model::{MediaError, Speaker, SpeechClient};

struct Captured {
    path: String,
    api_key: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

fn fake_elevenlabs(
    status: u16,
    response: Vec<u8>,
) -> (String, mpsc::Receiver<Captured>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake elevenlabs");
    let address = listener.local_addr().expect("address");
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept elevenlabs");
        let captured = read_request(&mut stream);
        sender.send(captured).expect("send captured");
        let reason = if status == 200 { "OK" } else { "Error" };
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: audio/ogg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        stream.write_all(head.as_bytes()).expect("head");
        stream.write_all(&response).expect("body");
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
    let mut api_key = None;
    let mut content_type = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).expect("header");
        if header == "\r\n" || header == "\n" || header.is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if lower.starts_with("content-length:") {
            content_length = header
                .split(':')
                .nth(1)
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
        }
        if lower.starts_with("xi-api-key:") {
            api_key = header
                .split_once(':')
                .map(|(_, value)| value.trim().to_owned());
        }
        if lower.starts_with("content-type:") {
            content_type = header
                .split_once(':')
                .map(|(_, value)| value.trim().to_owned());
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).expect("body");
    Captured {
        path,
        api_key,
        content_type,
        body,
    }
}

#[test]
fn speech_posts_elevenlabs_and_returns_audio() {
    let audio = vec![0x4f, 0x67, 0x67, 0x53, 0x00, 0x02];
    let (base, captured, server) = fake_elevenlabs(200, audio.clone());
    let client = SpeechClient::new(&base, "test-key", "voice-1", "eleven_v4", "opus_48000_64")
        .expect("client");

    let returned = client.speak("hello there").expect("speak");
    assert_eq!(returned, audio);

    let request = captured.recv().expect("captured");
    server.join().expect("server");
    assert_eq!(
        request.path,
        "/v1/text-to-speech/voice-1?output_format=opus_48000_64"
    );
    assert_eq!(request.api_key.as_deref(), Some("test-key"));
    assert_eq!(request.content_type.as_deref(), Some("application/json"));
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("json body");
    assert_eq!(body["text"], "hello there");
    assert_eq!(body["model_id"], "eleven_v4");
}

#[test]
fn a_speech_server_failure_is_retryable_for_429() {
    let (base, _captured, server) = fake_elevenlabs(429, b"slow down".to_vec());
    let client =
        SpeechClient::new(&base, "key", "voice", "eleven_v4", "opus_48000_64").expect("client");
    let error = client.speak("hello").expect_err("must fail");
    server.join().expect("server");
    assert!(matches!(error, MediaError::Response { status: 429, .. }));
    assert!(error.retryable());
}

#[test]
fn speech_preserves_elevenlabs_audio_tags() {
    let (base, captured, server) = fake_elevenlabs(200, b"OggS".to_vec());
    let client =
        SpeechClient::new(&base, "key", "voice", "eleven_v4", "opus_48000_64").expect("client");
    let script = "[warm, composed voice] Good evening. [softly] It is done. [laughs]";
    client.speak(script).expect("speak");
    let request = captured.recv().expect("captured");
    server.join().expect("server");
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("json body");
    assert_eq!(body["text"], script);
}
