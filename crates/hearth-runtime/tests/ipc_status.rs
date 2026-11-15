//! S8: the status usage window follows the configured zone.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Instant;

use hearth_core::{Clock, Harness, NewUsage, UuidV7};
use hearth_memory::MemoryService;
use hearth_model::Usage;
use hearth_runtime::{EventBus, StatusContext, Zone, read_status, serve};
use serde_json::Value;
use tempfile::TempDir;

const START: i64 = 1_768_519_800_000_000; // 2026-01-15T23:30:00Z
const ONE_HOUR: i64 = 3_600_000_000;
const INPUT_TOKENS: i64 = 1_200;
const CACHE_READ_TOKENS: i64 = 300;

#[derive(Clone)]
struct TestClock(Arc<AtomicI64>);

impl TestClock {
    fn new(start: i64) -> Self {
        Self(Arc::new(AtomicI64::new(start)))
    }

    fn advance(&self, micros: i64) {
        self.0.fetch_add(micros, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_micros(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn write_settings(home: &Path, zone: &str) {
    let path = home.join("settings.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .expect("open settings");
    write!(file, "{{\"identity\":{{\"timezone\":\"{zone}\"}}}}").expect("write settings");
    file.flush().expect("flush settings");
}

fn today_for(zone: &str) -> Value {
    let home = TempDir::new().expect("temp home");
    let clock = TestClock::new(START);
    let harness = Harness::open_with(home.path(), Arc::new(clock.clone()), Arc::new(UuidV7))
        .expect("harness");
    let session = harness.session().clone();
    session
        .record_usage(NewUsage {
            task_id: None,
            conversation_id: None,
            provider: "example".to_owned(),
            model: "fake-model".to_owned(),
            kind: "chat".to_owned(),
            usage: Usage {
                input_tokens: INPUT_TOKENS,
                cache_read_tokens: CACHE_READ_TOKENS,
                cache_write_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
            },
            latency_ms: 1,
        })
        .expect("usage");
    clock.advance(ONE_HOUR);
    write_settings(home.path(), zone);

    let context = StatusContext {
        session,
        model: "fake-model".to_owned(),
        started: Instant::now(),
        memory: Arc::new(MemoryService::new(harness.session().clone())),
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let handle = serve(
        home.path(),
        context,
        Arc::new(EventBus::new(Vec::new())),
        Arc::clone(&shutdown),
    )
    .expect("serve");
    let status = read_status(home.path()).expect("read status");
    shutdown.store(true, Ordering::SeqCst);
    handle.join().expect("server thread");
    status["usage"]["today"].clone()
}

#[test]
fn usage_today_starts_at_the_configured_local_midnight() {
    let utc = today_for("UTC");
    assert_eq!(utc["input_tokens"], 0, "UTC day excludes yesterday: {utc}");
    assert_eq!(utc["cache_read_tokens"], 0, "{utc}");

    if Zone::resolve("America/Sao_Paulo").name() != "America/Sao_Paulo" {
        // This host has no Sao Paulo entry, so the zone falls back to UTC.
        assert_eq!(today_for("America/Sao_Paulo")["input_tokens"], 0);
        return;
    }
    let local = today_for("America/Sao_Paulo");
    assert_eq!(
        local["input_tokens"], INPUT_TOKENS,
        "the local day still contains the event: {local}"
    );
    assert_eq!(local["cache_read_tokens"], CACHE_READ_TOKENS, "{local}");
}
