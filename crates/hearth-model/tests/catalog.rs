use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

#[test]
fn the_catalog_is_fetched_over_http() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let body = r#"{
        "example": {
            "api": "https://example.com/v1",
            "models": {
                "example-model": {
                    "limit": {"context": 64000, "output": 4096}
                }
            }
        }
    }"#;
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("request line");
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).expect("header");
            if header == "\r\n" || header == "\n" || header.is_empty() {
                break;
            }
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("response");
    });
    let catalog = hearth_model::fetch_catalog(&format!("{base}/api.json")).expect("catalog");
    assert_eq!(
        catalog
            .lookup("https://example.com/v1", "example-model")
            .expect("model")
            .context_window,
        64_000
    );
    server.join().expect("server");
}
