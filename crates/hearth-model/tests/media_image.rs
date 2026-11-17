use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use hearth_model::ImageClient;
use serde_json::json;

fn serve_json(body: &str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind image service");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let body = body.to_owned();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("request line");
        let mut length = 0usize;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).expect("header");
            if header == "\r\n" || header == "\n" || header.is_empty() {
                break;
            }
            if header.to_ascii_lowercase().starts_with("content-length:") {
                length = header
                    .split(':')
                    .nth(1)
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(0);
            }
        }
        let mut request = vec![0u8; length];
        reader.read_exact(&mut request).expect("request body");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("response");
    });
    (base, handle)
}

fn loopback_server() -> (String, Arc<AtomicUsize>, Arc<AtomicBool>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/secret", listener.local_addr().expect("addr"));
    let hits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_hits = Arc::clone(&hits);
    let thread_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    thread_hits.fetch_add(1, Ordering::SeqCst);
                    let body = b"not really an image";
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .expect("head");
                    stream.write_all(body).expect("body");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    (url, hits, stop, handle)
}

#[test]
fn a_service_supplied_loopback_url_is_refused() {
    let (secret_url, hits, stop, secret_thread) = loopback_server();
    let (base, service_thread) = serve_json(&json!({"data": [{"url": secret_url}]}).to_string());
    let client = ImageClient::new(base, "test-key", "test-model").expect("image client");

    let error = client
        .generate("a cat", None)
        .expect_err("a loopback download must be refused");
    assert!(
        error.to_string().contains("not public"),
        "unexpected error: {error}"
    );

    stop.store(true, Ordering::SeqCst);
    secret_thread.join().expect("secret server");
    service_thread.join().expect("image service");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the loopback listener must never be contacted"
    );
}
