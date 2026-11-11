use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

use hearth_model::{ProviderError, list_models};

fn serve_once(status: &'static str, body: &'static str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut request_line = String::new();
        reader.read_line(&mut request_line).expect("request line");
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("header line");
            if line == "\r\n" || line == "\n" || line.is_empty() {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
        request_line
    });
    (format!("http://{address}/v1"), handle)
}

#[test]
fn list_models_sorts_and_deduplicates_ids() {
    let (base_url, server) = serve_once(
        "200 OK",
        r#"{"data":[{"id":"zeta"},{"id":"alpha"},{"id":"zeta"}]}"#,
    );
    let models = list_models(&base_url, "fake-key").expect("list models");
    assert_eq!(models, vec!["alpha".to_owned(), "zeta".to_owned()]);
    let request = server.join().expect("server thread");
    assert!(
        request.starts_with("GET /v1/models "),
        "request was: {request}"
    );
}

#[test]
fn list_models_reports_a_provider_error() {
    let (base_url, server) = serve_once("401 Unauthorized", r#"{"error":"bad key"}"#);
    let error = list_models(&base_url, "fake-key").expect_err("reject");
    match error {
        ProviderError::Request { status, .. } => assert_eq!(status, 401),
        other => panic!("unexpected error: {other}"),
    }
    server.join().expect("server thread");
}

#[test]
fn list_models_accepts_a_top_level_array() {
    let (base_url, server) = serve_once("200 OK", r#"[{"id":"only"}]"#);
    let models = list_models(&base_url, "fake-key").expect("list models");
    assert_eq!(models, vec!["only".to_owned()]);
    server.join().expect("server thread");
}
