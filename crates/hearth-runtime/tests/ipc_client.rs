use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread::{self, JoinHandle};

use hearth_runtime::{IpcError, open_watch, read_status, socket_path};
use tempfile::TempDir;

fn serve(home: &Path, reply: &'static str) -> JoinHandle<()> {
    let listener = UnixListener::bind(socket_path(home)).expect("bind socket");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("request line");
        stream.write_all(reply.as_bytes()).expect("write reply");
        stream.flush().expect("flush reply");
    })
}

#[test]
fn read_status_returns_the_status_object() {
    let home = TempDir::new().expect("temp home");
    let server = serve(
        home.path(),
        "{\"v\":1,\"ok\":true,\"status\":{\"model\":\"fake-model\",\"uptime_seconds\":7}}\n",
    );
    let status = read_status(home.path()).expect("read status");
    assert_eq!(status["model"], "fake-model");
    assert_eq!(status["uptime_seconds"], 7);
    server.join().expect("server thread");
}

#[test]
fn read_status_reports_a_refusal() {
    let home = TempDir::new().expect("temp home");
    let server = serve(home.path(), "{\"v\":1,\"ok\":false,\"error\":\"busy\"}\n");
    let error = read_status(home.path()).expect_err("refused");
    match error {
        IpcError::Protocol(message) => assert_eq!(message, "busy"),
        other => panic!("unexpected error: {other}"),
    }
    server.join().expect("server thread");
}

#[test]
fn read_status_rejects_a_malformed_reply() {
    let home = TempDir::new().expect("temp home");
    let server = serve(home.path(), "not json\n");
    let error = read_status(home.path()).expect_err("malformed");
    assert!(matches!(error, IpcError::Protocol(_)), "{error}");
    server.join().expect("server thread");
}

#[test]
fn open_watch_returns_the_event_lines() {
    let home = TempDir::new().expect("temp home");
    let reply = "{\"v\":1,\"ok\":true}\n\
                 {\"v\":1,\"kind\":\"run_started\",\"data\":{}}\n\
                 {\"v\":1,\"kind\":\"run_settled\",\"data\":{\"status\":\"done\"}}\n";
    let server = serve(home.path(), reply);
    let reader = open_watch(home.path(), None).expect("open watch");
    let kinds: Vec<String> = reader
        .lines()
        .map(|line| {
            let value: serde_json::Value =
                serde_json::from_str(line.expect("event line").trim()).expect("event json");
            value["kind"].as_str().expect("kind").to_owned()
        })
        .collect();
    assert_eq!(kinds, vec!["run_started", "run_settled"]);
    server.join().expect("server thread");
}

#[test]
fn open_watch_reports_a_refusal() {
    let home = TempDir::new().expect("temp home");
    let server = serve(home.path(), "{\"v\":1,\"ok\":false,\"error\":\"no\"}\n");
    let error = open_watch(home.path(), None).expect_err("refused");
    assert!(matches!(error, IpcError::Protocol(_)), "{error}");
    server.join().expect("server thread");
}
