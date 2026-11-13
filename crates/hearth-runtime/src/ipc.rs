//! Private daemon socket for status and live event streams.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hearth_core::{CoreError, Session, UsageTotals};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use time::{Duration as TimeDuration, OffsetDateTime};

use crate::events::EventBus;

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

/// Values the socket server needs for a status reply.
#[derive(Clone)]
pub struct StatusContext {
    pub session: Session,
    pub model: String,
    pub started: Instant,
    pub memory: std::sync::Arc<hearth_memory::MemoryService>,
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
    let handle = thread::spawn(move || {
        while !shutdown.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let context = context.clone();
                    let bus = Arc::clone(&bus);
                    thread::spawn(move || {
                        let _ = handle_connection(stream, context, bus);
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
            let status = match status_value(&context) {
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

fn status_value(context: &StatusContext) -> Result<Value, IpcError> {
    let now = OffsetDateTime::now_utc();
    let midnight = now.date().midnight().assume_utc();
    let today = context
        .session
        .usage_totals_since(timestamp_micros(midnight))?;
    let week = context
        .session
        .usage_totals_since(timestamp_micros(now - TimeDuration::days(7)))?;
    let month = context
        .session
        .usage_totals_since(timestamp_micros(now - TimeDuration::days(30)))?;
    let total = context.session.usage_totals_since(0)?;
    let counts = context.session.task_counts()?;
    // Count the live store, not the retired table.
    let memory = context.memory.store().count().unwrap_or(0);
    let (standing_characters, standing_version) = context
        .memory
        .standing()
        .map(|(text, version)| (text.chars().count(), version))
        .unwrap_or((0, 0));
    let standing_bullets = context
        .memory
        .standing_store()
        .entries()
        .map(|items| items.len())
        .unwrap_or(0);
    let last_consolidation = context.session.meta_value("last_consolidation")?;
    let last_skill_review = context.session.meta_value("last_skill_review")?;
    let last_skill_review_changes = context
        .session
        .meta_value("last_skill_review_changes")?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
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
            "total": usage_value(total),
        },
        "cache_read_tokens": today.cache_read_tokens,
        "input_tokens": today.input_tokens,
        "output_tokens": today.output_tokens,
        "cache_hit_rate": cache_hit_rate,
        "memory_items": memory,
        "standing_memory_version": standing_version,
        "standing_bullets": standing_bullets,
        "standing_characters": standing_characters,
        "memory_recall": {
            "mode": context.memory.recall_mode(),
            "embeddings_configured": context.memory.embeddings_configured(),
            "embeddings_active": context.memory.embeddings_active(),
            "embedding_model": context.memory.embedding_model(),
            "max_items": context.memory.recall_max_items(),
            "max_tokens": context.memory.recall_max_tokens(),
        },
        "last_consolidation": last_consolidation,
        "last_skill_review": last_skill_review,
        "last_skill_review_changes": last_skill_review_changes,
    }))
}

fn usage_value(totals: UsageTotals) -> Value {
    json!({
        "input_tokens": totals.input_tokens,
        "output_tokens": totals.output_tokens,
        "cache_read_tokens": totals.cache_read_tokens,
    })
}

fn timestamp_micros(value: OffsetDateTime) -> i64 {
    value.unix_timestamp_nanos().div_euclid(1_000) as i64
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
