use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use hearth_runtime::{DoctorError, InitOptions, doctor_with, init};
use tempfile::TempDir;

fn serve_once(response: &'static str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
    let address = listener.local_addr().expect("server address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut buffer = [0u8; 8192];
        let _ = stream.read(&mut buffer);
        stream
            .write_all(response.as_bytes())
            .expect("write fake response");
    });
    (format!("http://{address}"), handle)
}

fn add_telegram_base(home: &std::path::Path, base: &str) {
    let path = home.join("settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read settings"))
            .expect("parse settings");
    settings["telegram"]["base_url"] = serde_json::json!(base);
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("settings"),
    )
    .expect("write");
}

fn init_home(home: &std::path::Path, provider_base: &str) {
    init(
        home,
        InitOptions {
            telegram_token: "fake-token".to_owned(),
            allowed_user_ids: vec![1],
            base_url: format!("{provider_base}/v1"),
            api_key: "fake-key".to_owned(),
            model: "fake-model".to_owned(),
            thinking: "default".to_owned(),
            agent_name: "Hearth".to_owned(),
            timezone: Some("UTC".to_owned()),
            media: None,
            context_window: None,
            daemon: hearth_config::Daemon::default(),
        },
    )
    .expect("init home");
}

#[test]
fn live_doctor_checks_telegram_and_provider_without_secrets() {
    let (telegram_base, telegram) = serve_once(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n\
         {\"ok\":true,\"result\":{\"id\":1,\"is_bot\":true,\"first_name\":\"Hearth\",\"username\":\"hearth_test\"}}",
    );
    let (provider_base, provider) = serve_once(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
         data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n\
         data: [DONE]\n\n",
    );
    let home = TempDir::new().expect("temp home");
    init_home(home.path(), &provider_base);
    add_telegram_base(home.path(), &telegram_base);

    let mut output = Vec::new();
    doctor_with(home.path(), true, &mut output).expect("live doctor");
    let text = String::from_utf8(output).expect("utf-8 output");
    assert!(
        text.contains("telegram: ok (@hearth_test)"),
        "output: {text}"
    );
    assert!(text.contains("provider: ok (fake-model)"), "output: {text}");
    assert!(!text.contains("fake-token"), "the token leaked: {text}");
    assert!(!text.contains("fake-key"), "the key leaked: {text}");

    telegram.join().expect("telegram server");
    provider.join().expect("provider server");
}

#[test]
fn live_doctor_reports_a_telegram_failure() {
    let (telegram_base, telegram) = serve_once(
        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n\
         {\"ok\":false,\"error_code\":401,\"description\":\"Unauthorized\"}",
    );
    let home = TempDir::new().expect("temp home");
    init_home(home.path(), "https://example.com");
    add_telegram_base(home.path(), &telegram_base);

    let mut output = Vec::new();
    let error = doctor_with(home.path(), true, &mut output).expect_err("telegram must fail");
    assert!(matches!(error, DoctorError::Telegram(_)), "error: {error}");
    telegram.join().expect("telegram server");
}

fn serve_capture(response: &'static str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
    let address = listener.local_addr().expect("server address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap_or(0);
        let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
        stream
            .write_all(response.as_bytes())
            .expect("write fake response");
        request
    });
    (format!("http://{address}"), handle)
}

#[test]
fn live_doctor_sends_the_configured_session_header() {
    let (provider_base, provider) = serve_capture(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
         data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n\
         data: [DONE]\n\n",
    );
    let home = TempDir::new().expect("temp home");
    init_home(home.path(), &provider_base);
    let path = home.path().join("settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("settings")).expect("json");
    settings
        .as_object_mut()
        .expect("settings object")
        .remove("telegram");
    settings["models"]["session_header"] = serde_json::json!("x-opencode-session");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("json"),
    )
    .expect("write");

    let mut output = Vec::new();
    doctor_with(home.path(), true, &mut output).expect("live doctor");
    let request = provider.join().expect("provider server");
    assert!(
        request.to_ascii_lowercase().contains("x-opencode-session:"),
        "request: {request}"
    );
    assert!(!request.contains("\"max_tokens\""), "request: {request}");
}
