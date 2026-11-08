use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    #[serde(default = "default_agent_name")]
    pub agent_name: String,
    #[serde(default = "default_user_name")]
    pub user_name: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default)]
    pub soul_file: Option<PathBuf>,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            agent_name: default_agent_name(),
            user_name: default_user_name(),
            timezone: default_timezone(),
            soul_file: None,
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
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub pricing: BTreeMap<String, PriceConfig>,
}

/// Per-model token prices in micros per million tokens.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceConfig {
    #[serde(default)]
    pub input: i64,
    #[serde(default)]
    pub cache_read: i64,
    #[serde(default)]
    pub cache_write: i64,
    #[serde(default)]
    pub output: i64,
    #[serde(default)]
    pub reasoning: i64,
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
    #[serde(default = "default_eta_reminder_minutes")]
    pub eta_reminder_minutes: u32,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            max_depth: default_max_depth(),
            retention_days: default_retention_days(),
            retention_runs: default_retention_runs(),
            eta_reminder_minutes: default_eta_reminder_minutes(),
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            workspace: None,
            max_turns: default_max_turns(),
        }
    }
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
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            standing_max_tokens: default_standing_max_tokens(),
            recall_max_tokens: default_recall_max_tokens(),
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

fn default_user_name() -> String {
    "User".to_owned()
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

fn default_eta_reminder_minutes() -> u32 {
    10
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

fn default_max_turns() -> u32 {
    30
}

fn default_context_window() -> u64 {
    128_000
}

fn default_max_output_tokens() -> u64 {
    8_192
}

fn default_soft_compact() -> f64 {
    0.65
}

fn default_hard_compact() -> f64 {
    0.85
}

fn default_keep_recent_tokens() -> u64 {
    24_000
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

fn default_log_level() -> String {
    "info".to_owned()
}
