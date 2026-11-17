//! Private daemon socket for status and live event streams.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hearth_core::{CoreError, Session, UsageTotals, Zone};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use time::Duration as TimeDuration;

use crate::events::EventBus;
use crate::skills::SkillLibrary;

/// The socket file inside the home folder.
pub const SOCKET_FILE: &str = "hearth.sock";

/// A socket client or server failure.
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("the daemon is not running: {0}")]
    Daemon(String),
    #[error("socket operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("socket protocol error: {0}")]
    Protocol(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// How long a status probe reuses the lifetime counters.
///
/// The TUI polls every two seconds. The lifetime usage total, the memory count,
/// the rules, and the skills index change slowly, and each one scans its whole
/// source, so they are kept for a few polls and never written by a read.
const STATUS_CACHE_TTL: Duration = Duration::from_secs(5);

/// The status fields that are expensive to recompute on every probe.
#[derive(Clone)]
struct SlowStatus {
    total: UsageTotals,
    memory_items: usize,
    memory_breakdown: hearth_memory::Breakdown,
    consolidation: Option<hearth_memory::ConsolidationReport>,
    rules: hearth_memory::Rules,
    last_consolidation: Option<String>,
    skills_index: String,
    skills_active: usize,
    last_skill_review: Option<String>,
    last_skill_review_changes: u64,
}

/// Values the socket server needs for a status reply.
#[derive(Clone)]
pub struct StatusContext {
    pub session: Session,
    pub model: String,
    pub started: Instant,
    pub memory: Arc<hearth_memory::MemoryService>,
    slow: Arc<Mutex<Option<(Instant, SlowStatus)>>>,
}

impl StatusContext {
    /// Build a status context with an empty counter cache.
    pub fn new(
        session: Session,
        model: String,
        started: Instant,
        memory: Arc<hearth_memory::MemoryService>,
    ) -> Self {
        Self {
            session,
            model,
            started,
            memory,
            slow: Arc::new(Mutex::new(None)),
        }
    }

    fn slow_status(&self, home: &Path) -> Result<SlowStatus, IpcError> {
        if let Ok(cache) = self.slow.lock()
            && let Some((at, value)) = cache.as_ref()
            && at.elapsed() < STATUS_CACHE_TTL
        {
            return Ok(value.clone());
        }
        let rules = self.memory.rules().unwrap_or_default();
        let library = SkillLibrary::new(home);
        let skills_index = library.index().unwrap_or_default();
        let value = SlowStatus {
            total: self.session.usage_totals_since(0)?,
            memory_items: self.memory.count().unwrap_or(0),
            memory_breakdown: self.memory.breakdown().unwrap_or_default(),
            consolidation: self
                .session
                .meta_value("last_consolidation_report")
                .ok()
                .flatten()
                .and_then(|text| serde_json::from_str(&text).ok()),
            rules,
            last_consolidation: self.session.meta_value("last_consolidation")?,
            skills_index,
            skills_active: library.stats().map(|stats| stats.active).unwrap_or(0),
            last_skill_review: self.session.meta_value("last_skill_review")?,
            last_skill_review_changes: self
                .session
                .meta_value("last_skill_review_changes")?
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
        };
        if let Ok(mut cache) = self.slow.lock() {
            *cache = Some((Instant::now(), value.clone()));
        }
        Ok(value)
    }
}

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    v: u32,
    #[serde(default)]
    cmd: String,
    #[serde(default)]
    task_id: Option<String>,
}

/// Bind the daemon socket and serve status and watch requests until shutdown.
pub fn serve(
    home: &Path,
    context: StatusContext,
    bus: Arc<EventBus>,
    shutdown: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, IpcError> {
    let path = socket_path(home);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let home = home.to_path_buf();
    let handle = thread::spawn(move || {
        while !shutdown.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let context = context.clone();
                    let bus = Arc::clone(&bus);
                    let home = home.clone();
                    thread::spawn(move || {
                        let _ = handle_connection(stream, &home, context, bus);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    tracing::warn!(%error, "daemon socket accept failed");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    });
    Ok(handle)
}

/// The socket path for a home.
pub fn socket_path(home: &Path) -> PathBuf {
    home.join(SOCKET_FILE)
}

fn handle_connection(
    mut stream: UnixStream,
    home: &Path,
    context: StatusContext,
    bus: Arc<EventBus>,
) -> Result<(), IpcError> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        return Ok(());
    }
    let request: Request = serde_json::from_str(&line)
        .map_err(|error| IpcError::Protocol(format!("invalid request: {error}")))?;
    if request.v != 1 {
        return write_error(&mut stream, "unsupported protocol version");
    }
    match request.cmd.as_str() {
        "status" => {
            let status = match status_value(home, &context) {
                Ok(status) => status,
                Err(error) => return write_error(&mut stream, &error.to_string()),
            };
            write_line(&mut stream, &json!({"v": 1, "ok": true, "status": status}))
        }
        "watch" => {
            write_line(&mut stream, &json!({"v": 1, "ok": true}))?;
            let receiver = bus.subscribe();
            loop {
                let Ok(event) = receiver.recv() else {
                    return Ok(());
                };
                if let Some(task_id) = request.task_id.as_deref() {
                    let value: Value = match serde_json::from_str(&event) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if value["task_id"].as_str() != Some(task_id) {
                        continue;
                    }
                }
                if stream.write_all(event.as_bytes()).is_err()
                    || stream.write_all(b"\n").is_err()
                    || stream.flush().is_err()
                {
                    return Ok(());
                }
            }
        }
        other => write_error(&mut stream, &format!("unknown command: {other}")),
    }
}

fn status_value(home: &Path, context: &StatusContext) -> Result<Value, IpcError> {
    let now = context.session.now_micros();
    let zone = Zone::resolve(&hearth_config::identity_timezone(home));
    let today = context.session.usage_totals_since(zone.start_of_day(now))?;
    let week = context
        .session
        .usage_totals_since(now - TimeDuration::days(7).whole_microseconds() as i64)?;
    let month = context
        .session
        .usage_totals_since(now - TimeDuration::days(30).whole_microseconds() as i64)?;
    let slow = context.slow_status(home)?;
    let counts = context.session.task_counts()?;
    let cache_hit_rate = hearth_core::cache_hit_rate(today.input_tokens, today.cache_read_tokens);
    Ok(json!({
        "model": context.model,
        "uptime_seconds": context.started.elapsed().as_secs(),
        "tasks_running": counts.running,
        "tasks_asleep": counts.asleep,
        "usage": {
            "today": usage_value(today),
            "week": usage_value(week),
            "month": usage_value(month),
            "total": usage_value(slow.total),
        },
        "cache_read_tokens": today.cache_read_tokens,
        "input_tokens": today.input_tokens,
        "output_tokens": today.output_tokens,
        "cache_hit_rate": cache_hit_rate,
        "memory_items": slow.memory_items,
        "memory": {
            "pinned": slow.memory_breakdown.pinned,
            "active": slow.memory_breakdown.active,
            "retired": slow.memory_breakdown.retired,
            "audit": slow
                .consolidation
                .as_ref()
                .map(hearth_memory::ConsolidationReport::summary_json),
        },
        "rules_version": slow.rules.version,
        "rules_bullets": slow.rules.count,
        "rules_characters": slow.rules.text.chars().count(),
        "memory_recall": {
            "mode": context.memory.mode(),
            "embeddings_configured": context.memory.embeddings_configured(),
            "embeddings_active": context.memory.embeddings_active(),
            "embedding_model": context.memory.embedding_model(),
            "max_items": context.memory.config().recall_max_items,
        },
        "last_consolidation": slow.last_consolidation,
        "skills": {
            "active": slow.skills_active,
            "index_tokens": hearth_model::estimate_tokens(&slow.skills_index),
        },
        "last_skill_review": slow.last_skill_review,
        "last_skill_review_changes": slow.last_skill_review_changes,
    }))
}

fn usage_value(totals: UsageTotals) -> Value {
    json!({
        "input_tokens": totals.input_tokens,
        "output_tokens": totals.output_tokens,
        "cache_read_tokens": totals.cache_read_tokens,
    })
}

fn write_line(stream: &mut UnixStream, value: &Value) -> Result<(), IpcError> {
    let text = serde_json::to_string(value)
        .map_err(|error| IpcError::Protocol(format!("could not encode reply: {error}")))?;
    stream.write_all(text.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn write_error(stream: &mut UnixStream, message: &str) -> Result<(), IpcError> {
    write_line(stream, &json!({"v": 1, "ok": false, "error": message}))
}

/// Connect to a running daemon.
fn connect(home: &Path) -> Result<UnixStream, IpcError> {
    let path = socket_path(home);
    let stream = UnixStream::connect(&path)
        .map_err(|error| IpcError::Daemon(format!("{} ({error})", path.display())))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}

/// Read the status object from a running daemon.
pub fn read_status(home: &Path) -> Result<Value, IpcError> {
    let mut stream = connect(home)?;
    writeln!(stream, "{}", json!({"v": 1, "cmd": "status"}))?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|error| IpcError::Protocol(format!("invalid status reply: {error}")))?;
    if value["ok"] != true {
        return Err(IpcError::Protocol(
            value["error"]
                .as_str()
                .unwrap_or("status was refused")
                .to_owned(),
        ));
    }
    Ok(value["status"].clone())
}

/// Open the live event stream from a running daemon.
///
/// The returned reader blocks while the stream is idle; drop it to detach.
pub fn open_watch(home: &Path, task_id: Option<String>) -> Result<BufReader<UnixStream>, IpcError> {
    let mut stream = connect(home)?;
    let request = json!({"v": 1, "cmd": "watch", "task_id": task_id});
    writeln!(stream, "{request}")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let ack: Value = serde_json::from_str(line.trim())
        .map_err(|error| IpcError::Protocol(format!("invalid watch ack: {error}")))?;
    if ack["ok"] != true {
        return Err(IpcError::Protocol(
            ack["error"]
                .as_str()
                .unwrap_or("watch was refused")
                .to_owned(),
        ));
    }
    reader.get_ref().set_read_timeout(None)?;
    Ok(reader)
}
