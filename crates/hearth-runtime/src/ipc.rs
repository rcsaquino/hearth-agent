//! Private daemon socket for status and live event streams.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hearth_core::{CoreError, Session};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use time::OffsetDateTime;

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
    let midnight = OffsetDateTime::now_utc()
        .date()
        .midnight()
        .assume_utc()
        .unix_timestamp()
        * 1_000_000;
    let totals = context.session.usage_totals_since(midnight)?;
    let counts = context.session.task_counts()?;
    let memory = context.session.memory_item_count()?;
    let standing_version = context.memory.standing_version().unwrap_or(0);
    let last_consolidation = context.session.meta_value("last_consolidation")?;
    let last_skill_review = context.session.meta_value("last_skill_review")?;
    let last_skill_review_changes = context
        .session
        .meta_value("last_skill_review_changes")?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let denominator = totals.input_tokens + totals.cache_read_tokens;
    let cache_hit_rate = if denominator > 0 {
        totals.cache_read_tokens as f64 / denominator as f64
    } else {
        0.0
    };
    Ok(json!({
        "model": context.model,
        "uptime_seconds": context.started.elapsed().as_secs(),
        "tasks_running": counts.running,
        "tasks_asleep": counts.asleep,
        "today_cost_micros": totals.cost_micros,
        "cache_read_tokens": totals.cache_read_tokens,
        "input_tokens": totals.input_tokens,
        "cache_hit_rate": cache_hit_rate,
        "memory_items": memory,
        "standing_memory_version": standing_version,
        "last_consolidation": last_consolidation,
        "last_skill_review": last_skill_review,
        "last_skill_review_changes": last_skill_review_changes,
    }))
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
    UnixStream::connect(&path)
        .map_err(|error| IpcError::Daemon(format!("{} ({error})", path.display())))
}

/// Watch live events from a running daemon.
pub fn watch(home: &Path, task_id: Option<String>, json: bool) -> Result<(), IpcError> {
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
    if json {
        println!("{}", line.trim());
    } else {
        println!("[watch] connected");
    }
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if json {
            println!("{line}");
            continue;
        }
        print_human_event(&line);
    }
    Ok(())
}

fn print_human_event(line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        println!("{line}");
        return;
    };
    let kind = value["kind"].as_str().unwrap_or_default();
    let data = &value["data"];
    match kind {
        "text_delta" => {
            print!("{}", data["text"].as_str().unwrap_or_default());
            let _ = std::io::stdout().flush();
        }
        "thinking_delta" => {
            print!("[thinking] {}", data["text"].as_str().unwrap_or_default());
            let _ = std::io::stdout().flush();
        }
        "tool_call_started" => println!(
            "\n[tool] {} {}",
            data["name"].as_str().unwrap_or_default(),
            data["arguments"].as_str().unwrap_or_default()
        ),
        "tool_result" => println!(
            "\n[result] {}",
            data["content"].as_str().unwrap_or_default()
        ),
        "usage" => println!(
            "\n[usage] input={} cache={} output={} cost_micros={}",
            data["input_tokens"],
            data["cache_read_tokens"],
            data["output_tokens"],
            data["cost_micros"]
        ),
        "run_started" => println!("\n[run] started"),
        "turn_started" => println!("[run] turn {}", data["turn"]),
        "task_step" => println!(
            "[run] {} step {}",
            data["phase"].as_str().unwrap_or_default(),
            data["turn"]
        ),
        "run_settled" => println!(
            "\n[settled] {}",
            data["status"].as_str().unwrap_or_default()
        ),
        _ => println!("[{kind}] {data}"),
    }
}

/// Ask a running daemon for its status.
pub fn status(home: &Path, json: bool) -> Result<(), IpcError> {
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
    let status = &value["status"];
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(status).unwrap_or_else(|_| status.to_string())
        );
        return Ok(());
    }
    println!("model: {}", status["model"].as_str().unwrap_or_default());
    println!("uptime: {}s", status["uptime_seconds"]);
    println!(
        "tasks: {} running, {} asleep",
        status["tasks_running"], status["tasks_asleep"]
    );
    if let Some(value) = status["last_consolidation"].as_str() {
        println!("last consolidation: {value}");
    }
    if let Some(value) = status["last_skill_review"].as_str() {
        println!("last skill review: {value}");
    }
    println!(
        "last review changes: {}",
        status["last_skill_review_changes"]
    );
    println!(
        "today cost: {:.4}",
        status["today_cost_micros"].as_i64().unwrap_or(0) as f64 / 1_000_000.0
    );
    println!(
        "cache hit: {:.1}%",
        status["cache_hit_rate"].as_f64().unwrap_or(0.0) * 100.0
    );
    println!("memory items: {}", status["memory_items"]);
    Ok(())
}
