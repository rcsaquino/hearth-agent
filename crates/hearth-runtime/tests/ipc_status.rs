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

    let context = StatusContext::new(
        session,
        "fake-model".to_owned(),
        Instant::now(),
        Arc::new(MemoryService::new(harness.session().clone())),
    );
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

fn write_skill(home: &Path, learned: bool, name: &str, description: &str) {
    let path = if learned {
        home.join("skills/learned").join(format!("{name}.md"))
    } else {
        home.join("skills").join(format!("{name}.md"))
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("skill directory");
    }
    let status = if learned {
        "version: 1\nstatus: active\ncreated: 2026-01-01\nupdated: 2026-01-01\nlast_used: \nuses: 0\nsuccesses: 0\nfailures: 0\nevidence: [entry-1]\n"
    } else {
        ""
    };
    std::fs::write(
        &path,
        format!(
            "---\nname: {name}\ndescription: {description}\n{status}---\n\n## Steps\n\nDo it.\n"
        ),
    )
    .expect("skill file");
}

#[test]
fn status_reports_the_active_skill_count_and_index_cost() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    write_skill(home.path(), false, "writing", "Write clearly");
    write_skill(home.path(), true, "research", "Research a topic");
    let expected = hearth_runtime::skills::SkillLibrary::new(home.path())
        .index()
        .expect("index");
    let expected_tokens = hearth_model::estimate_tokens(&expected);

    let context = StatusContext::new(
        harness.session().clone(),
        "fake-model".to_owned(),
        Instant::now(),
        Arc::new(MemoryService::new(harness.session().clone())),
    );
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

    assert_eq!(status["skills"]["active"], 2, "{status}");
    assert_eq!(
        status["skills"]["index_tokens"], expected_tokens,
        "the status must measure the same index the agent builds: {status}"
    );
}

#[test]
fn status_reports_the_memory_breakdown_and_the_last_audit() {
    use hearth_memory::{Kind, NewMemory};

    let home = TempDir::new().expect("temp home");
    write_settings(home.path(), "UTC");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session().clone();
    let memory = MemoryService::new(session.clone());
    memory
        .write(NewMemory::new("Pinned rule.").kind(Kind::Rule).pinned(true))
        .expect("rule");
    memory
        .write(NewMemory::new("the user prefers tea").subject("user"))
        .expect("active");
    let retired = memory
        .write(NewMemory::new("Retired fact.").subject("retired"))
        .expect("retired")
        .memory()
        .clone();
    memory
        .write(
            NewMemory::new("Replacement fact.")
                .subject("retired")
                .supersedes(retired.id),
        )
        .expect("replacement");
    let now = session.now_micros();
    let content = "the user prefers tea";
    let fingerprint = hearth_memory::fingerprint(content);
    session
        .storage()
        .write(move |writer| {
            writer.execute(
                "INSERT INTO memories (id, kind, content, subject, terms, fingerprint, pinned, \
                 source_kind, source_entry, valid_from, valid_to, created_at, updated_at, revision) \
                 VALUES ('m_dup', 'preference', ?, 'user', '', ?, 0, 'user', NULL, ?, NULL, ?, ?, 1)",
                hearth_storage::rusqlite::params![content, fingerprint, now, now, now],
            )?;
            Ok::<_, hearth_memory::MemoryError>(())
        })
        .expect("seed duplicate");
    memory.consolidate().expect("consolidate");

    let context = StatusContext::new(
        session,
        "fake-model".to_owned(),
        Instant::now(),
        Arc::new(memory),
    );
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

    assert_eq!(status["memory"]["pinned"], 1, "{status}");
    assert_eq!(status["memory"]["active"], 2, "{status}");
    assert_eq!(
        status["memory"]["retired"], 2,
        "the closed duplicate is retired too: {status}"
    );
    assert_eq!(status["memory"]["audit"]["canonical_closed"], 1, "{status}");
    assert_eq!(status["memory"]["audit"]["near_duplicates"], 0, "{status}");
    assert_eq!(status["memory_items"], 3, "{status}");
    assert!(status["last_consolidation"].as_str().is_some(), "{status}");
}
