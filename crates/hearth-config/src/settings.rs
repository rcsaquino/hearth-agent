use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The whole settings file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub identity: Identity,
    #[serde(default)]
    pub telegram: Option<Telegram>,
    #[serde(default)]
    pub models: Option<Models>,
    #[serde(default)]
    pub media: Option<Media>,
    #[serde(default)]
    pub daemon: Daemon,
    #[serde(default)]
    pub tasks: Tasks,
    #[serde(default)]
    pub web: Option<Web>,
    #[serde(default)]
    pub agent: Agent,
    #[serde(default)]
    pub compaction: Compaction,
    #[serde(default)]
    pub memory: Memory,
    #[serde(default)]
    pub logging: Logging,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Daemon {
    #[serde(default = "default_daemon_start")]
    pub start: String,
    #[serde(default = "default_keep_running")]
    pub keep_running: bool,
}

impl Default for Daemon {
    fn default() -> Self {
        Self {
            start: default_daemon_start(),
            keep_running: default_keep_running(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    #[serde(default = "default_agent_name")]
    pub agent_name: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            agent_name: default_agent_name(),
            timezone: default_timezone(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Telegram {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub allowed_user_ids: Vec<i64>,
    #[serde(default)]
    pub allowed_chat_ids: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    pub kind: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u64,
    #[serde(default)]
    pub session_header: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    #[serde(default)]
    pub image: Option<ImageService>,
    #[serde(default)]
    pub stt: Option<SttService>,
    #[serde(default)]
    pub tts: Option<TtsService>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageService {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SttService {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsService {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub voice_id: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tasks {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u8,
    #[serde(default = "default_max_depth")]
    pub max_depth: u8,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_retention_runs")]
    pub retention_runs: u32,
    #[serde(default = "default_eta_reminder_seconds")]
    pub eta_reminder_seconds: u32,
    #[serde(default)]
    pub eta_reminder_minutes: Option<u32>,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            max_depth: default_max_depth(),
            retention_days: default_retention_days(),
            retention_runs: default_retention_runs(),
            eta_reminder_seconds: default_eta_reminder_seconds(),
            eta_reminder_minutes: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Web {
    #[serde(default)]
    pub search: WebSearch,
    #[serde(default)]
    pub fetch: WebFetch,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebFetch {
    #[serde(default = "default_fetch_order")]
    pub order: Vec<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, FetchProvider>,
}

impl Default for WebFetch {
    fn default() -> Self {
        Self {
            order: default_fetch_order(),
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchProvider {
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearch {
    #[serde(default = "default_search_order")]
    pub order: Vec<String>,
    #[serde(default = "default_search_parallel")]
    pub parallel: u8,
    #[serde(default = "default_search_results")]
    pub results: u8,
    #[serde(default = "default_search_max_results")]
    pub max_results: u8,
    #[serde(default)]
    pub providers: BTreeMap<String, SearchProvider>,
}

impl Default for WebSearch {
    fn default() -> Self {
        Self {
            order: default_search_order(),
            parallel: default_search_parallel(),
            results: default_search_results(),
            max_results: default_search_max_results(),
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchProvider {
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    #[serde(default)]
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compaction {
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u64,
    #[serde(default = "default_idle_min_tokens")]
    pub idle_min_tokens: u64,
    #[serde(default = "default_soft_compact")]
    pub soft_compact: f64,
    #[serde(default = "default_hard_compact")]
    pub hard_compact: f64,
    #[serde(default = "default_keep_recent_tokens")]
    pub keep_recent_tokens: u64,
}

impl Default for Compaction {
    fn default() -> Self {
        Self {
            idle_minutes: default_idle_minutes(),
            idle_min_tokens: default_idle_min_tokens(),
            soft_compact: default_soft_compact(),
            hard_compact: default_hard_compact(),
            keep_recent_tokens: default_keep_recent_tokens(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    #[serde(default = "default_standing_max_tokens")]
    pub standing_max_tokens: u64,
    #[serde(default = "default_recall_max_tokens")]
    pub recall_max_tokens: u64,
    #[serde(default = "default_recall_max_items")]
    pub recall_max_items: u64,
    /// Semantic recall. Off means keyword search only.
    #[serde(default = "default_embeddings")]
    pub embeddings: bool,
    /// Directory holding `model.onnx` and `tokenizer.json`.
    #[serde(default = "default_embedding_model_dir")]
    pub embedding_model_dir: String,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            standing_max_tokens: default_standing_max_tokens(),
            recall_max_tokens: default_recall_max_tokens(),
            recall_max_items: default_recall_max_items(),
            embeddings: default_embeddings(),
            embedding_model_dir: default_embedding_model_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logging {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

fn default_agent_name() -> String {
    "Hearth".to_owned()
}

fn default_daemon_start() -> String {
    "tui".to_owned()
}

fn default_keep_running() -> bool {
    true
}

fn default_timezone() -> String {
    "UTC".to_owned()
}

fn default_max_concurrent() -> u8 {
    4
}

fn default_max_depth() -> u8 {
    2
}

fn default_retention_days() -> u32 {
    30
}

fn default_retention_runs() -> u32 {
    25
}

fn default_eta_reminder_seconds() -> u32 {
    1
}

fn default_fetch_order() -> Vec<String> {
    vec!["jina".to_owned(), "http".to_owned()]
}

fn default_search_order() -> Vec<String> {
    ["exa", "brave", "deepseek", "firecrawl", "tinyfish"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn default_search_parallel() -> u8 {
    3
}

fn default_search_results() -> u8 {
    5
}

fn default_search_max_results() -> u8 {
    10
}

fn default_context_window() -> u64 {
    128_000
}

fn default_max_output_tokens() -> u64 {
    32_000
}

fn default_soft_compact() -> f64 {
    0.65
}

fn default_hard_compact() -> f64 {
    0.85
}

fn default_keep_recent_tokens() -> u64 {
    20_000
}

fn default_idle_minutes() -> u64 {
    60
}

fn default_idle_min_tokens() -> u64 {
    100_000
}

fn default_standing_max_tokens() -> u64 {
    1200
}

fn default_recall_max_tokens() -> u64 {
    900
}

fn default_recall_max_items() -> u64 {
    8
}

fn default_embeddings() -> bool {
    true
}

fn default_embedding_model_dir() -> String {
    "models/bekko-embedding-v1-a8m".to_owned()
}

fn default_log_level() -> String {
    "info".to_owned()
}

/// The canonical defaults shown by the settings surface.
///
/// Optional sections are present as partial objects so a row can still compare
/// its value with the effective default when the section itself is absent.
pub fn default_settings_value() -> Value {
    json!({
        "identity": {
            "agent_name": default_agent_name(),
            "timezone": default_timezone(),
        },
        "telegram": {
            "base_url": null,
            "bot_token": null,
            "allowed_user_ids": [],
            "allowed_chat_ids": [],
        },
        "models": {
            "kind": "openai-compatible",
            "context_window": default_context_window(),
            "max_output_tokens": default_max_output_tokens(),
            "session_header": null,
            "thinking": "default",
            "headers": {},
        },
        "media": {},
        "daemon": {
            "start": default_daemon_start(),
            "keep_running": default_keep_running(),
        },
        "tasks": {
            "max_concurrent": default_max_concurrent(),
            "max_depth": default_max_depth(),
            "retention_days": default_retention_days(),
            "retention_runs": default_retention_runs(),
            "eta_reminder_seconds": default_eta_reminder_seconds(),
        },
        "web": {
            "search": {
                "order": default_search_order(),
                "parallel": default_search_parallel(),
                "results": default_search_results(),
                "max_results": default_search_max_results(),
                "providers": {},
            },
            "fetch": {
                "order": default_fetch_order(),
                "providers": {},
            },
        },
        "agent": {
            "workspace": null,
        },
        "compaction": {
            "idle_minutes": default_idle_minutes(),
            "idle_min_tokens": default_idle_min_tokens(),
            "soft_compact": default_soft_compact(),
            "hard_compact": default_hard_compact(),
            "keep_recent_tokens": default_keep_recent_tokens(),
        },
        "memory": {
            "standing_max_tokens": default_standing_max_tokens(),
            "recall_max_tokens": default_recall_max_tokens(),
        },
        "logging": {"level": default_log_level()},
    })
}

/// Keep only values that differ from the canonical defaults.
///
/// Required model fields stay even when they match a default so the result still
/// deserializes into `Models`.
pub fn compact_settings(value: &Value) -> Value {
    let defaults = default_settings_value();
    compact_value(value, Some(&defaults), &[]).unwrap_or_else(|| Value::Object(Map::new()))
}

fn compact_value(value: &Value, default: Option<&Value>, path: &[String]) -> Option<Value> {
    if path == ["identity", "soul_file"]
        || (path.len() >= 2 && path[0] == "models" && path[1] == "pricing")
    {
        return None;
    }
    match value {
        Value::Object(object) => {
            let mut compact = Map::new();
            for (key, child) in object {
                let mut next = path.to_vec();
                next.push(key.clone());
                let child_default = default.and_then(|default| default.get(key));
                if let Some(child) = compact_value(child, child_default, &next) {
                    compact.insert(key.clone(), child);
                }
            }
            if compact.is_empty() && !path.is_empty() {
                None
            } else {
                Some(Value::Object(compact))
            }
        }
        Value::Array(items) => {
            if default.is_some_and(|default| default == value) || items.is_empty() {
                return None;
            }
            Some(value.clone())
        }
        Value::Null => match default {
            Some(default) if !default.is_null() => Some(Value::Null),
            _ => None,
        },
        Value::String(text) if text.is_empty() => match default {
            Some(default) if default.as_str().is_some_and(|value| !value.is_empty()) => {
                Some(value.clone())
            }
            _ => None,
        },
        _ => {
            if required_model_field(path) {
                return Some(value.clone());
            }
            if default.is_some_and(|default| default == value) {
                None
            } else {
                Some(value.clone())
            }
        }
    }
}

fn required_model_field(path: &[String]) -> bool {
    path.len() == 2
        && path[0] == "models"
        && matches!(path[1].as_str(), "kind" | "base_url" | "api_key" | "model")
}
