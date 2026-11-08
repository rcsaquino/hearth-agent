use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use hearth_model::{ImageClient, MediaError, SpeechClient, Transcriber, TranscriptionClient};
use hearth_tools::{MediaTool, Tool};
use serde_json::{Value, json};
use tempfile::TempDir;

struct EchoTranscriber;

impl Transcriber for EchoTranscriber {
    fn transcribe(&self, _path: &Path) -> Result<String, MediaError> {
        Ok("transcribed".to_owned())
    }
}

const TINY_PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl7S4sAAAAASUVORK5CYII=";

fn roots() -> (TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
    let directory = TempDir::new().expect("temp directory");
    let workspace = directory.path().join("workspace");
    let spill = directory.path().join("spill");
    let media = directory.path().join("media/inbound");
    let outbound = directory.path().join("media/outbound");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&spill).expect("spill");
    fs::create_dir_all(&media).expect("media");
    fs::create_dir_all(&outbound).expect("outbound");
    (directory, workspace, spill, media, outbound)
}

fn server(response: Vec<u8>, content_type: &str) -> (String, JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let content_type = content_type.to_owned();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut request_line = String::new();
        reader.read_line(&mut request_line).expect("request line");
        let mut headers = String::new();
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("header");
            if line == "\r\n" || line == "\n" || line.is_empty() {
                break;
            }
            if line.to_ascii_lowercase().starts_with("content-length:") {
                length = line
                    .split(':')
                    .nth(1)
                    .unwrap_or_default()
                    .trim()
                    .parse()
                    .unwrap_or(0);
            }
            headers.push_str(&line);
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).expect("body");
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        stream.write_all(head.as_bytes()).expect("head");
        stream.write_all(&response).expect("response");
        stream.flush().expect("flush");
        json!({
            "request_line": request_line.trim(),
            "headers": headers,
            "body": String::from_utf8_lossy(&body),
        })
    });
    (format!("http://127.0.0.1:{port}"), handle)
}

#[test]
fn image_generation_writes_a_private_png() {
    let response = json!({"data": [{"b64_json": TINY_PNG_B64}]}).to_string();
    let (base, handle) = server(response.into_bytes(), "application/json");
    let image = ImageClient::new(base, "fake-key", "image-model").expect("image client");
    let (_directory, workspace, spill, media, outbound) = roots();
    let tool = MediaTool::new(
        Some(image),
        None,
        None,
        &workspace,
        &spill,
        &media,
        &outbound,
    );

    let output = tool
        .call(&json!({"action": "image", "prompt": "a red cube", "size": "1024x1024"}))
        .expect("image generation");
    let path = output
        .content
        .strip_prefix("image: ")
        .expect("image path prefix");
    let bytes = fs::read(path).expect("image bytes");
    assert!(bytes.starts_with(b"\x89PNG"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    let request = handle.join().expect("server thread");
    assert!(
        request["request_line"]
            .as_str()
            .unwrap_or_default()
            .contains("/images/generations")
    );
    assert!(
        request["headers"]
            .as_str()
            .unwrap_or_default()
            .contains("authorization: Bearer fake-key")
            || request["headers"]
                .as_str()
                .unwrap_or_default()
                .contains("Authorization: Bearer fake-key")
    );
    let body: Value =
        serde_json::from_str(request["body"].as_str().unwrap_or_default()).expect("request body");
    assert_eq!(body["model"], "image-model");
    assert_eq!(body["prompt"], "a red cube");
}

#[test]
fn transcription_reads_the_workspace_file() {
    let (base, handle) = server(br#"{"text":"hello voice"}"#.to_vec(), "application/json");
    let transcriber = TranscriptionClient::new(base, "fake-key", "whisper-large-v3-turbo")
        .expect("transcription client");
    let (_directory, workspace, spill, media, outbound) = roots();
    fs::write(workspace.join("voice.ogg"), b"OggS").expect("voice file");
    let tool = MediaTool::new(
        None,
        Some(Arc::new(transcriber)),
        None,
        &workspace,
        &spill,
        &media,
        &outbound,
    );

    let output = tool
        .call(&json!({"action": "transcribe", "path": "voice.ogg"}))
        .expect("transcription");
    assert_eq!(output.content, "hello voice");
    let request = handle.join().expect("server thread");
    assert!(
        request["headers"]
            .as_str()
            .unwrap_or_default()
            .contains("multipart/form-data")
    );
}

#[test]
fn speech_writes_a_private_ogg() {
    let (base, handle) = server(b"OggS fake opus".to_vec(), "audio/ogg");
    let speaker = SpeechClient::new(base, "fake-key", "voice-id", "eleven_v4", "opus_48000_64")
        .expect("speech client");
    let (_directory, workspace, spill, media, outbound) = roots();
    let tool = MediaTool::new(
        None,
        None,
        Some(Arc::new(speaker)),
        &workspace,
        &spill,
        &media,
        &outbound,
    );

    let output = tool
        .call(&json!({"action": "speak", "text": "hello"}))
        .expect("speech");
    let path = output
        .content
        .strip_prefix("voice: ")
        .expect("voice path prefix");
    assert_eq!(fs::read(path).expect("audio"), b"OggS fake opus");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let request = handle.join().expect("server thread");
    assert!(
        request["request_line"]
            .as_str()
            .unwrap_or_default()
            .contains("/v1/text-to-speech/voice-id")
    );
}

#[test]
fn missing_configuration_is_a_clear_tool_error() {
    let (_directory, workspace, spill, media, outbound) = roots();
    let tool = MediaTool::new(None, None, None, &workspace, &spill, &media, &outbound);

    for action in ["image", "transcribe", "speak"] {
        let error = tool
            .call(&json!({"action": action, "prompt": "x", "text": "x", "path": "x"}))
            .expect_err("missing media configuration");
        assert_eq!(error.code, "media_not_configured");
    }
}

#[test]
fn media_paths_stay_confined() {
    let (_directory, workspace, spill, media, outbound) = roots();
    let tool = MediaTool::new(
        None,
        Some(Arc::new(EchoTranscriber)),
        None,
        &workspace,
        &spill,
        &media,
        &outbound,
    );
    let error = tool
        .call(&json!({"action": "transcribe", "path": "../outside.ogg"}))
        .expect_err("escape");
    assert_eq!(error.code, "invalid_path");
}
