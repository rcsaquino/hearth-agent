use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use hearth_tools::{
    BraveSearch, DeepSeekSearch, ExaSearch, FirecrawlSearch, SearchProvider, TinyFishSearch, Tool,
    WebSearchTool,
};
use serde_json::{Value, json};

struct Captured {
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

struct Server {
    base: String,
    thread: JoinHandle<Captured>,
}

struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
}

fn serve(reply: Reply) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
    let base = format!("http://{}", listener.local_addr().expect("local addr"));
    let thread = thread::spawn(move || {
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
        let body = String::from_utf8_lossy(&body).to_string();
        let response = format!(
            "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            reply.status,
            reply.content_type,
            reply.body.len(),
            reply.body
        );
        stream.write_all(response.as_bytes()).expect("write reply");
        Captured {
            path,
            headers,
            body,
        }
    });
    Server { base, thread }
}

fn sse(payload: Value) -> Reply {
    Reply {
        status: 200,
        content_type: "text/event-stream",
        body: format!("data: {payload}\n\n"),
    }
}

fn json_reply(payload: Value) -> Reply {
    Reply {
        status: 200,
        content_type: "application/json",
        body: payload.to_string(),
    }
}

fn exa_mcp_payload() -> Value {
    let inner = json!({
        "results": [{
            "title": "Example",
            "url": "https://example.com/a",
            "publishedDate": "2026-01-02T00:00:00.000Z",
            "highlights": ["A useful snippet"]
        }]
    });
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "content": [{"type": "text", "text": inner.to_string()}]
        }
    })
}

#[test]
fn exa_mcp_streams_and_maps_results() {
    let server = serve(sse(exa_mcp_payload()));
    let provider: Arc<dyn SearchProvider> = Arc::new(
        ExaSearch::new(server.base.clone(), server.base.clone(), None).expect("exa provider"),
    );
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["rust"]}))
        .expect("search call");
    let value: Value = serde_json::from_str(&output.content).expect("json output");
    assert_eq!(value["results"][0]["title"], "Example");
    assert_eq!(value["results"][0]["url"], "https://example.com/a");
    assert_eq!(value["results"][0]["snippet"], "A useful snippet");
    assert_eq!(value["results"][0]["date"], "2026-01-02T00:00:00.000Z");

    let captured = server.thread.join().expect("server thread");
    assert_eq!(captured.path, "/mcp?tools=web_search_advanced_exa");
    let request: Value = serde_json::from_str(&captured.body).expect("json rpc body");
    assert_eq!(request["method"], "tools/call");
    assert_eq!(request["params"]["name"], "web_search_advanced_exa");
    assert_eq!(request["params"]["arguments"]["query"], "rust");
    assert_eq!(request["params"]["arguments"]["numResults"], 5);
}

#[test]
fn exa_rest_uses_the_api_key_and_maps_results() {
    let server = serve(json_reply(json!({
        "results": [{
            "title": "Rest Example",
            "url": "https://example.com/rest",
            "publishedDate": "2026-02-03",
            "text": "Rest snippet"
        }]
    })));
    let provider: Arc<dyn SearchProvider> = Arc::new(
        ExaSearch::new(
            server.base.clone(),
            server.base.clone(),
            Some("exa-key".to_owned()),
        )
        .expect("exa provider"),
    );
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["rust"]}))
        .expect("search call");
    let value: Value = serde_json::from_str(&output.content).expect("json output");
    assert_eq!(value["results"][0]["title"], "Rest Example");
    assert_eq!(value["results"][0]["snippet"], "Rest snippet");

    let captured = server.thread.join().expect("server thread");
    assert_eq!(captured.path, "/search");
    assert_eq!(
        captured.headers.get("x-api-key").map(String::as_str),
        Some("exa-key")
    );
}

#[test]
fn brave_maps_web_results() {
    let server = serve(json_reply(json!({
        "web": {"results": [{
            "title": "Brave Example",
            "url": "https://brave.example/a",
            "description": "Brave snippet",
            "age": "2 days ago"
        }]}
    })));
    let provider: Arc<dyn SearchProvider> = Arc::new(
        BraveSearch::new(server.base.clone(), "brave-key".to_owned()).expect("brave provider"),
    );
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["rust"]}))
        .expect("search call");
    let value: Value = serde_json::from_str(&output.content).expect("json output");
    assert_eq!(value["results"][0]["title"], "Brave Example");
    assert_eq!(value["results"][0]["snippet"], "Brave snippet");
    assert_eq!(value["results"][0]["date"], "2 days ago");

    let captured = server.thread.join().expect("server thread");
    assert!(captured.path.starts_with("/web/search?"));
    assert!(captured.path.contains("q=rust"));
    assert!(captured.path.contains("count=5"));
    assert_eq!(
        captured
            .headers
            .get("x-subscription-token")
            .map(String::as_str),
        Some("brave-key")
    );
}

#[test]
fn deepseek_maps_results() {
    let server = serve(json_reply(json!({
        "results": [{
            "title": "DeepSeek Example",
            "url": "https://deepseek.example/a",
            "summary": "DeepSeek snippet"
        }]
    })));
    let provider: Arc<dyn SearchProvider> = Arc::new(
        DeepSeekSearch::new(server.base.clone(), "ds-key".to_owned()).expect("deepseek provider"),
    );
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["rust"]}))
        .expect("search call");
    let value: Value = serde_json::from_str(&output.content).expect("json output");
    assert_eq!(value["results"][0]["title"], "DeepSeek Example");
    assert_eq!(value["results"][0]["snippet"], "DeepSeek snippet");

    let captured = server.thread.join().expect("server thread");
    assert_eq!(captured.path, "/v1/tools/search");
    assert_eq!(
        captured.headers.get("authorization").map(String::as_str),
        Some("Bearer ds-key")
    );
    let body: Value = serde_json::from_str(&captured.body).expect("request body");
    assert_eq!(body["query"], "rust");
    assert_eq!(body["count"], 5);
}

#[test]
fn firecrawl_maps_web_results() {
    let server = serve(json_reply(json!({
        "success": true,
        "data": {"web": [{
            "title": "Firecrawl Example",
            "url": "https://firecrawl.example/a",
            "description": "Firecrawl snippet"
        }]}
    })));
    let provider: Arc<dyn SearchProvider> = Arc::new(
        FirecrawlSearch::new(server.base.clone(), "fc-key".to_owned()).expect("firecrawl provider"),
    );
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["rust"]}))
        .expect("search call");
    let value: Value = serde_json::from_str(&output.content).expect("json output");
    assert_eq!(value["results"][0]["title"], "Firecrawl Example");
    assert_eq!(value["results"][0]["snippet"], "Firecrawl snippet");

    let captured = server.thread.join().expect("server thread");
    assert_eq!(captured.path, "/v2/search");
    assert_eq!(
        captured.headers.get("authorization").map(String::as_str),
        Some("Bearer fc-key")
    );
    let body: Value = serde_json::from_str(&captured.body).expect("request body");
    assert_eq!(body["query"], "rust");
    assert_eq!(body["limit"], 5);
    assert_eq!(body["sources"][0], "web");
}

#[test]
fn tinyfish_maps_results() {
    let server = serve(json_reply(json!({
        "results": [{
            "title": "TinyFish Example",
            "url": "https://tinyfish.example/a",
            "snippet": "TinyFish snippet",
            "date": "2026-03-04"
        }]
    })));
    let provider: Arc<dyn SearchProvider> = Arc::new(
        TinyFishSearch::new(server.base.clone(), "tf-key".to_owned()).expect("tinyfish provider"),
    );
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["rust"]}))
        .expect("search call");
    let value: Value = serde_json::from_str(&output.content).expect("json output");
    assert_eq!(value["results"][0]["title"], "TinyFish Example");
    assert_eq!(value["results"][0]["snippet"], "TinyFish snippet");
    assert_eq!(value["results"][0]["date"], "2026-03-04");

    let captured = server.thread.join().expect("server thread");
    assert!(captured.path.contains("query=rust"));
    assert_eq!(
        captured.headers.get("x-api-key").map(String::as_str),
        Some("tf-key")
    );
}

struct ScriptedSearch {
    results: HashMap<String, Result<Vec<hearth_tools::SearchResult>, hearth_tools::SearchError>>,
}

impl SearchProvider for ScriptedSearch {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn search(
        &self,
        query: &str,
        _limit: usize,
    ) -> Result<Vec<hearth_tools::SearchResult>, hearth_tools::SearchError> {
        self.results
            .get(query)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

fn result(title: &str, url: &str) -> hearth_tools::SearchResult {
    hearth_tools::SearchResult {
        title: title.to_owned(),
        url: url.to_owned(),
        snippet: String::new(),
        date: None,
    }
}

#[test]
fn a_failed_provider_falls_through_to_the_next() {
    let failing = serve(Reply {
        status: 503,
        content_type: "text/plain",
        body: "down".to_owned(),
    });
    let healthy = serve(json_reply(json!({
        "web": {"results": [{
            "title": "Fallback",
            "url": "https://fallback.example",
            "description": "from brave"
        }]}
    })));
    let exa: Arc<dyn SearchProvider> =
        Arc::new(ExaSearch::new(failing.base.clone(), failing.base.clone(), None).expect("exa"));
    let brave: Arc<dyn SearchProvider> =
        Arc::new(BraveSearch::new(healthy.base.clone(), "brave-key".to_owned()).expect("brave"));
    let tool = WebSearchTool::new(vec![exa, brave], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["rust"]}))
        .expect("fallback search");
    let value: Value = serde_json::from_str(&output.content).expect("json output");
    assert_eq!(value["results"][0]["url"], "https://fallback.example");
    let _ = failing.thread.join().expect("failing server");
    let _ = healthy.thread.join().expect("healthy server");
}

#[test]
fn results_are_deduplicated_and_capped() {
    let mut results = HashMap::new();
    results.insert(
        "a".to_owned(),
        Ok(vec![
            result("one", "https://one.example"),
            result("two", "https://two.example"),
        ]),
    );
    results.insert(
        "b".to_owned(),
        Ok(vec![
            result("one again", "https://one.example"),
            result("three", "https://three.example"),
        ]),
    );
    let provider: Arc<dyn SearchProvider> = Arc::new(ScriptedSearch { results });
    let tool = WebSearchTool::new(vec![provider], 3, 5, 2);

    let output = tool.call(&json!({"queries": ["a", "b"]})).expect("search");
    let value: Value = serde_json::from_str(&output.content).expect("json");
    assert_eq!(value["results"].as_array().expect("results").len(), 2);
    assert_eq!(value["results"][0]["url"], "https://one.example");
    assert_eq!(value["results"][1]["url"], "https://two.example");
}

#[test]
fn every_provider_failing_is_a_tool_error() {
    let mut results = HashMap::new();
    results.insert(
        "a".to_owned(),
        Err(hearth_tools::SearchError::temporary("provider down")),
    );
    let provider: Arc<dyn SearchProvider> = Arc::new(ScriptedSearch { results });
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let error = tool
        .call(&json!({"queries": ["a"]}))
        .expect_err("all failed");
    assert_eq!(error.code, "search_failed");
    assert!(error.message.contains("provider down"));
    assert!(error.retryable);
}

#[test]
fn a_partial_failure_keeps_the_successful_results() {
    let mut results = HashMap::new();
    results.insert(
        "good".to_owned(),
        Ok(vec![result("Good", "https://good.example")]),
    );
    results.insert(
        "bad".to_owned(),
        Err(hearth_tools::SearchError::permanent("bad query")),
    );
    let provider: Arc<dyn SearchProvider> = Arc::new(ScriptedSearch { results });
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    let output = tool
        .call(&json!({"queries": ["good", "bad"]}))
        .expect("partial search");
    let value: Value = serde_json::from_str(&output.content).expect("json");
    assert_eq!(value["results"][0]["url"], "https://good.example");
    assert_eq!(value["errors"][0]["query"], "bad");
}

#[test]
fn invalid_query_arguments_are_rejected() {
    let provider: Arc<dyn SearchProvider> = Arc::new(ScriptedSearch {
        results: HashMap::new(),
    });
    let tool = WebSearchTool::new(vec![provider], 3, 5, 10);

    for arguments in [json!({}), json!({"queries": []}), json!({"queries": [""]})] {
        let error = tool.call(&arguments).expect_err("invalid queries");
        assert_eq!(error.code, "invalid_arguments");
    }

    let eleven: Vec<String> = (0..11).map(|index| format!("q{index}")).collect();
    let error = tool
        .call(&json!({"queries": eleven}))
        .expect_err("too many queries");
    assert_eq!(error.code, "invalid_arguments");
}

#[test]
fn each_query_is_capped_to_the_requested_results() {
    let mut results = HashMap::new();
    results.insert(
        "a".to_owned(),
        Ok(vec![
            result("one", "https://one.example"),
            result("two", "https://two.example"),
        ]),
    );
    let provider: Arc<dyn SearchProvider> = Arc::new(ScriptedSearch { results });
    let tool = WebSearchTool::new(vec![provider], 3, 1, 10);

    let output = tool.call(&json!({"queries": ["a"]})).expect("search");
    let value: Value = serde_json::from_str(&output.content).expect("json");
    assert_eq!(value["results"].as_array().expect("results").len(), 1);
}
