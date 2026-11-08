use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use hearth_tools::{
    DeepSeekFetch, FetchError, FetchProvider, FetchResult, FirecrawlFetch, HttpFetch, JinaFetch,
    TinyFishFetch, Tool, WebFetchTool,
};
use serde_json::json;

struct Captured {
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    content_type: Option<&'static str>,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

struct Server {
    base: String,
    thread: JoinHandle<Vec<Captured>>,
}

fn reply(status: u16, content_type: &'static str, body: impl Into<Vec<u8>>) -> Reply {
    Reply {
        status,
        content_type: Some(content_type),
        headers: Vec::new(),
        body: body.into(),
    }
}

fn redirect(location: &str) -> Reply {
    Reply {
        status: 302,
        content_type: None,
        headers: vec![("Location", location.to_owned())],
        body: Vec::new(),
    }
}

fn serve(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
    let base = format!("http://{}", listener.local_addr().expect("local addr"));
    let thread = thread::spawn(move || {
        let mut captured = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            reader.read_line(&mut request_line).expect("request line");
            let path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let mut headers = HashMap::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header line");
                if line == "\r\n" || line == "\n" || line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
                }
            }
            let length = headers
                .get("content-length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let mut body = vec![0u8; length];
            if length > 0 {
                reader.read_exact(&mut body).expect("request body");
            }
            let mut response = format!(
                "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                reply.status,
                reply.body.len()
            );
            if let Some(content_type) = reply.content_type {
                response.push_str(&format!("Content-Type: {content_type}\r\n"));
            }
            for (name, value) in &reply.headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str("\r\n");
            stream
                .write_all(response.as_bytes())
                .expect("write headers");
            stream.write_all(&reply.body).expect("write body");
            captured.push(Captured {
                path,
                headers,
                body,
            });
        }
        captured
    });
    Server { base, thread }
}

fn target(server: &Server, path: &str) -> String {
    format!("{}{}", server.base, path)
}

#[test]
fn jina_is_tried_first_and_labels_untrusted_content() {
    let jina = serve(vec![reply(200, "text/markdown", "# Hello\n\nWorld")]);
    let provider = Arc::new(JinaFetch::new(jina.base.clone(), None).expect("jina"));
    let calls = Arc::new(AtomicUsize::new(0));
    let fallback = Arc::new(CountingProvider {
        calls: Arc::clone(&calls),
    });
    let tool = WebFetchTool::new(vec![provider, fallback], false);

    let output = tool
        .call(&json!({"url": "https://93.184.216.34/page"}))
        .expect("fetch");
    assert!(
        output.content.contains("Provider: jina"),
        "{}",
        output.content
    );
    assert!(output.content.contains("URL: https://93.184.216.34/page"));
    assert!(
        output
            .content
            .contains("[Untrusted web content from https://93.184.216.34/page.")
    );
    assert!(output.content.contains("# Hello"));
    assert!(output.content.contains("World"));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the fallback ran");

    let captured = jina.thread.join().expect("jina thread");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].path, "/https://93.184.216.34/page");
}

#[test]
fn jina_sends_an_optional_api_key() {
    let jina = serve(vec![reply(200, "text/plain", "content")]);
    let provider =
        Arc::new(JinaFetch::new(jina.base.clone(), Some("jina-key".to_owned())).expect("jina"));
    let tool = WebFetchTool::new(vec![provider], false);
    tool.call(&json!({"url": "https://93.184.216.34/page"}))
        .expect("fetch");
    let captured = jina.thread.join().expect("jina thread");
    assert_eq!(
        captured[0].headers.get("authorization").map(String::as_str),
        Some("Bearer jina-key")
    );
}

#[test]
fn jina_failure_falls_back_to_direct_html() {
    let jina = serve(vec![reply(500, "text/plain", "down")]);
    let direct = serve(vec![reply(
        200,
        "text/html",
        "<html><body><h1>Hello</h1><script>bad()</script><style>.x{color:red}</style><p>World</p></body></html>",
    )]);
    let target = target(&direct, "/page");

    let providers: Vec<Arc<dyn FetchProvider>> = vec![
        Arc::new(JinaFetch::new(jina.base.clone(), None).expect("jina")),
        Arc::new(
            HttpFetch::new("http", true, 2 * 1024 * 1024, 3, Duration::from_secs(5))
                .expect("direct"),
        ),
    ];
    let tool = WebFetchTool::new(providers, true);
    let output = tool.call(&json!({"url": target})).expect("fetch");
    assert!(
        output.content.contains("Provider: http"),
        "{}",
        output.content
    );
    assert!(output.content.contains("Hello"));
    assert!(output.content.contains("World"));
    assert!(!output.content.contains("bad()"));
    assert!(!output.content.contains(".x{color:red}"));

    let jina_captured = jina.thread.join().expect("jina thread");
    assert!(jina_captured[0].path.contains("127.0.0.1"));
    let direct_captured = direct.thread.join().expect("direct thread");
    assert_eq!(direct_captured[0].path, "/page");
}

#[test]
fn direct_pdf_text_is_extracted() {
    let direct = serve(vec![reply(
        200,
        "application/pdf",
        include_bytes!("fixtures/sample.pdf").to_vec(),
    )]);
    let target = target(&direct, "/sample.pdf");
    let provider = Arc::new(
        HttpFetch::new("http", true, 2 * 1024 * 1024, 3, Duration::from_secs(5)).expect("direct"),
    );
    let tool = WebFetchTool::new(vec![provider], true);
    let output = tool.call(&json!({"url": target})).expect("fetch");
    assert!(
        output.content.contains("Hello Hearth PDF"),
        "{}",
        output.content
    );
    direct.thread.join().expect("direct thread");
}

#[test]
fn redirects_are_followed_to_the_final_url() {
    let direct = serve(vec![redirect("/next"), reply(200, "text/plain", "done")]);
    let target = target(&direct, "/start");
    let provider = Arc::new(
        HttpFetch::new("http", true, 2 * 1024 * 1024, 3, Duration::from_secs(5)).expect("direct"),
    );
    let tool = WebFetchTool::new(vec![provider], true);
    let output = tool.call(&json!({"url": target})).expect("fetch");
    assert!(output.content.contains("done"));
    assert!(output.content.contains("/next"), "{}", output.content);
    let captured = direct.thread.join().expect("direct thread");
    assert_eq!(captured[0].path, "/start");
    assert_eq!(captured[1].path, "/next");
}

#[test]
fn too_many_redirects_are_refused() {
    let direct = serve(vec![
        redirect("/one"),
        redirect("/two"),
        redirect("/three"),
        redirect("/four"),
    ]);
    let target = target(&direct, "/start");
    let provider = Arc::new(
        HttpFetch::new("http", true, 2 * 1024 * 1024, 3, Duration::from_secs(5)).expect("direct"),
    );
    let tool = WebFetchTool::new(vec![provider], true);
    let error = tool.call(&json!({"url": target})).expect_err("redirects");
    assert!(
        error.message.contains("too many redirects"),
        "{}",
        error.message
    );
    direct.thread.join().expect("direct thread");
}

#[test]
fn the_page_cap_truncates_and_notes_it() {
    let direct = serve(vec![reply(200, "text/plain", "a".repeat(100))]);
    let target = target(&direct, "/big");
    let provider =
        Arc::new(HttpFetch::new("http", true, 32, 3, Duration::from_secs(5)).expect("direct"));
    let tool = WebFetchTool::new(vec![provider], true);
    let output = tool.call(&json!({"url": target})).expect("fetch");
    assert!(output.content.contains("[page truncated at 32 bytes]"));
    direct.thread.join().expect("direct thread");
}

struct CountingProvider {
    calls: Arc<AtomicUsize>,
}

impl FetchProvider for CountingProvider {
    fn name(&self) -> &'static str {
        "counting"
    }

    fn fetch(&self, _url: &str) -> Result<FetchResult, FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(FetchResult {
            text: "should not run".to_owned(),
            final_url: "https://example.com".to_owned(),
            title: None,
        })
    }
}

#[test]
fn blocked_targets_are_refused_before_any_provider_runs() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(CountingProvider {
        calls: Arc::clone(&calls),
    });
    let tool = WebFetchTool::new(vec![provider], false);
    for url in [
        "http://10.0.0.1/",
        "http://169.254.1.1/",
        "http://127.0.0.1/",
        "ftp://example.com/",
    ] {
        let error = tool.call(&json!({"url": url})).expect_err("blocked");
        assert!(!error.message.is_empty(), "{url}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn an_unsupported_content_type_is_an_error() {
    let direct = serve(vec![reply(200, "application/x-unknown", "abc")]);
    let target = target(&direct, "/file");
    let provider = Arc::new(
        HttpFetch::new("http", true, 2 * 1024 * 1024, 3, Duration::from_secs(5)).expect("direct"),
    );
    let tool = WebFetchTool::new(vec![provider], true);
    let error = tool
        .call(&json!({"url": target}))
        .expect_err("content type");
    assert!(
        error.message.contains("unsupported content type"),
        "{}",
        error.message
    );
    direct.thread.join().expect("direct thread");
}

#[test]
fn every_provider_failing_returns_the_first_error() {
    let first = serve(vec![reply(404, "text/plain", "missing")]);
    let second = serve(vec![reply(500, "text/plain", "down")]);
    let target = target(&second, "/page");
    let providers: Vec<Arc<dyn FetchProvider>> = vec![
        Arc::new(JinaFetch::new(first.base.clone(), None).expect("jina")),
        Arc::new(
            HttpFetch::new("http", true, 2 * 1024 * 1024, 3, Duration::from_secs(5))
                .expect("direct"),
        ),
    ];
    let tool = WebFetchTool::new(providers, true);
    let error = tool.call(&json!({"url": target})).expect_err("all fail");
    assert!(error.message.contains("404"), "{}", error.message);
    first.thread.join().expect("first thread");
    second.thread.join().expect("second thread");
}

#[test]
fn a_large_error_body_is_shortened() {
    let direct = serve(vec![reply(500, "text/plain", "x".repeat(1000))]);
    let target = target(&direct, "/error");
    let provider = Arc::new(
        HttpFetch::new("http", true, 2 * 1024 * 1024, 3, Duration::from_secs(5)).expect("direct"),
    );
    let tool = WebFetchTool::new(vec![provider], true);
    let error = tool
        .call(&json!({"url": target}))
        .expect_err("server error");
    assert!(
        error.message.len() < 300,
        "message was {} bytes",
        error.message.len()
    );
    direct.thread.join().expect("direct thread");
}

#[test]
fn tinyfish_maps_its_fetch_response() {
    let server = serve(vec![reply(
        200,
        "application/json",
        r##"{"results":[{"url":"https://93.184.216.34/page","final_url":"https://93.184.216.34/page","title":"Tiny Page","text":"# Tiny\n\nContent"}],"errors":[]}"##,
    )]);
    let provider = Arc::new(
        TinyFishFetch::new(
            "tiny-key",
            server.base.clone(),
            Duration::from_secs(5),
            2 * 1024 * 1024,
        )
        .expect("tinyfish"),
    );
    let tool = WebFetchTool::new(vec![provider], false);
    let output = tool
        .call(&json!({"url": "https://93.184.216.34/page"}))
        .expect("fetch");
    assert!(output.content.contains("Provider: tinyfish"));
    assert!(output.content.contains("Title: Tiny Page"));
    assert!(output.content.contains("# Tiny"));
    assert!(output.content.contains("Content"));

    let captured = server.thread.join().expect("server thread");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].path, "/");
    assert_eq!(
        captured[0].headers.get("x-api-key").map(String::as_str),
        Some("tiny-key")
    );
    let body: serde_json::Value =
        serde_json::from_slice(&captured[0].body).expect("request body is JSON");
    assert_eq!(body["urls"], json!(["https://93.184.216.34/page"]));
    assert_eq!(body["format"], "markdown");
    assert_eq!(body["per_url_timeout_ms"], 30000);
}

#[test]
fn firecrawl_maps_its_scrape_response() {
    let server = serve(vec![reply(
        200,
        "application/json",
        r##"{"success":true,"data":{"markdown":"# Fire\n\nContent","metadata":{"title":"Fire Page"}}}"##,
    )]);
    let provider = Arc::new(
        FirecrawlFetch::new(
            "fire-key",
            server.base.clone(),
            Duration::from_secs(5),
            2 * 1024 * 1024,
        )
        .expect("firecrawl"),
    );
    let tool = WebFetchTool::new(vec![provider], false);
    let output = tool
        .call(&json!({"url": "https://93.184.216.34/page"}))
        .expect("fetch");
    assert!(output.content.contains("Provider: firecrawl"));
    assert!(output.content.contains("Title: Fire Page"));
    assert!(output.content.contains("# Fire"));
    assert!(output.content.contains("Content"));

    let captured = server.thread.join().expect("server thread");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].path, "/v2/scrape");
    assert_eq!(
        captured[0].headers.get("authorization").map(String::as_str),
        Some("Bearer fire-key")
    );
    let body: serde_json::Value =
        serde_json::from_slice(&captured[0].body).expect("request body is JSON");
    assert_eq!(body["url"], "https://93.184.216.34/page");
    assert_eq!(body["formats"], json!(["markdown"]));
    assert_eq!(body["onlyMainContent"], true);
    assert_eq!(body["lockdown"], true);
}

#[test]
fn deepseek_maps_its_fetch_response() {
    let server = serve(vec![reply(
        200,
        "application/json",
        r#"{"title":"Deep Page","content":"Deep content"}"#,
    )]);
    let provider = Arc::new(
        DeepSeekFetch::new(
            "deep-key",
            server.base.clone(),
            Duration::from_secs(5),
            2 * 1024 * 1024,
        )
        .expect("deepseek"),
    );
    let tool = WebFetchTool::new(vec![provider], false);
    let output = tool
        .call(&json!({"url": "https://93.184.216.34/page"}))
        .expect("fetch");
    assert!(output.content.contains("Provider: deepseek"));
    assert!(output.content.contains("Title: Deep Page"));
    assert!(output.content.contains("Deep content"));

    let captured = server.thread.join().expect("server thread");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].path, "/v1/tools/fetch");
    assert_eq!(
        captured[0].headers.get("authorization").map(String::as_str),
        Some("Bearer deep-key")
    );
    let body: serde_json::Value =
        serde_json::from_slice(&captured[0].body).expect("request body is JSON");
    assert_eq!(body["url"], "https://93.184.216.34/page");
}

#[test]
fn a_keyed_provider_is_used_after_jina_fails() {
    let jina = serve(vec![reply(500, "text/plain", "down")]);
    let tiny = serve(vec![reply(
        200,
        "application/json",
        r##"{"results":[{"url":"https://93.184.216.34/page","title":"Tiny","text":"from tinyfish"}],"errors":[]}"##,
    )]);
    let providers: Vec<Arc<dyn FetchProvider>> = vec![
        Arc::new(JinaFetch::new(jina.base.clone(), None).expect("jina")),
        Arc::new(
            TinyFishFetch::new(
                "tiny-key",
                tiny.base.clone(),
                Duration::from_secs(5),
                2 * 1024 * 1024,
            )
            .expect("tinyfish"),
        ),
    ];
    let tool = WebFetchTool::new(providers, false);
    let output = tool
        .call(&json!({"url": "https://93.184.216.34/page"}))
        .expect("fetch");
    assert!(output.content.contains("Provider: tinyfish"));
    assert!(output.content.contains("from tinyfish"));
    jina.thread.join().expect("jina thread");
    tiny.thread.join().expect("tinyfish thread");
}

#[test]
fn a_keyed_provider_401_is_not_retryable() {
    let server = serve(vec![reply(401, "text/plain", "unauthorized")]);
    let provider = TinyFishFetch::new(
        "tiny-key",
        server.base.clone(),
        Duration::from_secs(5),
        2 * 1024 * 1024,
    )
    .expect("tinyfish");
    let error = provider
        .fetch("https://93.184.216.34/page")
        .expect_err("unauthorized");
    assert!(!error.retryable, "401 was marked retryable");
    assert!(error.message.contains("401"), "{}", error.message);
    server.thread.join().expect("server thread");
}

#[test]
fn keyed_providers_failing_together_return_the_first_error() {
    let tiny = serve(vec![reply(500, "text/plain", "tiny down")]);
    let fire = serve(vec![reply(500, "text/plain", "fire down")]);
    let providers: Vec<Arc<dyn FetchProvider>> = vec![
        Arc::new(
            TinyFishFetch::new(
                "tiny-key",
                tiny.base.clone(),
                Duration::from_secs(5),
                2 * 1024 * 1024,
            )
            .expect("tinyfish"),
        ),
        Arc::new(
            FirecrawlFetch::new(
                "fire-key",
                fire.base.clone(),
                Duration::from_secs(5),
                2 * 1024 * 1024,
            )
            .expect("firecrawl"),
        ),
    ];
    let tool = WebFetchTool::new(providers, false);
    let error = tool
        .call(&json!({"url": "https://93.184.216.34/page"}))
        .expect_err("all fail");
    assert!(error.message.contains("500"), "{}", error.message);
    tiny.thread.join().expect("tinyfish thread");
    fire.thread.join().expect("firecrawl thread");
}
