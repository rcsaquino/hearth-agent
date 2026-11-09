//! The deterministic 100-turn token-efficiency benchmark.
//!
//! The scripted provider simulates a provider prefix cache honestly: only the
//! stable prefix is cached, and only when its hash is unchanged. The benchmark
//! reports the section 14 targets next to the measured values.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hearth_core::{
    Clock, ContextBudget, Harness, PromptInput, Save, UuidV7, build_prompt, summary_text,
};
use hearth_memory::{MemoryKind, MemoryService, MemoryTool, NewMemoryInput, SourceKind};
use hearth_model::{
    ChatMessage, ChatRequest, ContentBlock, Pricing, Provider, ProviderError, ProviderStream, Role,
    StreamEvent, ToolCall, ToolSpec, Usage, estimate_tokens,
};
use hearth_runtime::{Agent, AgentConfig, ToolGroups, ToolSearchTool};
use hearth_tools::{
    ExaSearch, FetchProvider, FileTool, HttpFetch, JinaFetch, SearchProvider, ShellTool, Tool,
    ToolRegistry, WebFetchTool, WebSearchTool,
};
use serde::Serialize;
use serde_json::{Value, json};
use tempfile::TempDir;

const TURNS: usize = 100;
const FIXED_NOW: i64 = 1_735_689_600_000_000; // 2025-01-01T00:00:00Z
const WINDOW: u64 = 8_192;
const MAX_OUTPUT: u64 = 1_024;
const KEEP_RECENT: u64 = 2_000;
const TARGET_FIXED: u64 = 4_500;
const TARGET_CACHE: f64 = 0.80;
const TARGET_SAVE_MS: f64 = 15.0;
const TARGET_ASSEMBLY_MS: f64 = 300.0;
const TARGET_RECALL_MS: f64 = 80.0;

struct FixedClock;

impl Clock for FixedClock {
    fn now_micros(&self) -> i64 {
        FIXED_NOW
    }
}

/// The measured 100-turn benchmark.
#[derive(Debug, Clone, Serialize)]
pub struct TokensReport {
    pub turns: usize,
    pub compactions: usize,
    pub summary_tokens: u64,
    pub fixed_overhead_p95: u64,
    pub fixed_overhead_max: u64,
    pub prefix_tokens_p95: u64,
    pub prompt_tokens_p95: u64,
    pub prefix_stable: bool,
    pub cache_read_share: f64,
    pub cache_hit_turns: usize,
    pub save_ms_p95: f64,
    pub assembly_ms_p95: f64,
    pub recall_ms_p95: f64,
    pub token_totals: BTreeMap<String, u64>,
    pub per_turn: Vec<TurnReport>,
    pub targets: Vec<TargetReport>,
    pub deferred: Vec<String>,
}

/// One turn's prompt and cache numbers.
#[derive(Debug, Clone, Serialize)]
pub struct TurnReport {
    pub turn: usize,
    pub total: u64,
    pub fixed: u64,
    pub prefix: u64,
    pub cache_read: u64,
}

/// One measured target.
#[derive(Debug, Clone, Serialize)]
pub struct TargetReport {
    pub name: String,
    pub measured: f64,
    pub target: f64,
    pub unit: String,
    pub met: bool,
}

/// Run the 100-turn script and return the measured report.
pub fn run() -> Result<TokensReport> {
    let dir = TempDir::new().context("create the token benchmark home")?;
    let harness = Harness::open_with(dir.path(), Arc::new(FixedClock), Arc::new(UuidV7))
        .context("open the token benchmark record")?;
    let session = harness.session().clone();
    let memory = Arc::new(MemoryService::new(session.clone()));
    memory
        .save(NewMemoryInput {
            content: "Sam prefers metric units and short replies.".to_owned(),
            kind: MemoryKind::Preference,
            tags: vec!["preference".to_owned()],
            importance: 90,
            source_kind: SourceKind::User,
            source_ref: None,
            supersedes: None,
            valid_from: None,
            valid_to: None,
        })
        .context("seed the preference")?;

    let workspace = dir.path().join("workspace");
    fs::create_dir_all(&workspace).context("create the workspace")?;
    fs::write(workspace.join("notes.txt"), "Project Atlas notes.\n").context("write notes")?;
    let spill = dir.path().join("spill");

    let groups = Arc::new(ToolGroups::default());
    let selection = Arc::new(Mutex::new(HashSet::new()));
    let search = WebSearchTool::new(
        vec![Arc::new(ExaSearch::new(
            "https://mcp.exa.ai",
            "https://api.exa.ai",
            None,
        )?) as Arc<dyn SearchProvider>],
        3,
        5,
        10,
    );
    let fetch = WebFetchTool::new(
        vec![
            Arc::new(JinaFetch::new("https://r.jina.ai", None)?) as Arc<dyn FetchProvider>,
            Arc::new(HttpFetch::production("http")?) as Arc<dyn FetchProvider>,
        ],
        false,
    );
    let tool_list: Vec<Arc<dyn Tool>> = vec![
        Arc::new(FileTool::new(workspace.clone(), spill.clone())),
        Arc::new(ShellTool::new(workspace.clone(), Duration::from_secs(60))),
        Arc::new(search),
        Arc::new(fetch),
        Arc::new(MemoryTool::new(memory.clone())),
        Arc::new(ToolSearchTool::new(groups.clone(), selection.clone())),
    ];
    let tools = ToolRegistry::new(tool_list);
    let tool_specs = tools.specs();

    let provider = Arc::new(EfficiencyProvider::new());
    let budget = ContextBudget::new(WINDOW, MAX_OUTPUT, 0.65, 0.85, KEEP_RECENT);
    let soul = "You are Hearth, a personal agent for one user.".to_owned();
    let model = "eval-model".to_owned();
    let agent = Agent::new(
        session.clone(),
        provider.clone(),
        Pricing::default(),
        AgentConfig {
            soul: soul.clone(),
            model: model.clone(),
            provider_name: "eval".to_owned(),
            owner: "tokens-bench".to_owned(),
            max_turns: 30,
            tools,
            spill_dir: spill.clone(),
            voice_tags: false,
            guidance: String::new(),
        },
    )
    .with_memory(memory.clone())
    .with_budget(budget)
    .with_groups(groups.clone(), selection.clone());

    let conversation = session
        .create_conversation("Tokens", "local", "local:tokens")
        .context("create the conversation")?
        .id;
    let timing = session
        .create_conversation("Timing", "local", "local:timing")
        .context("create the timing conversation")?
        .id;

    let mut fixed = Vec::with_capacity(TURNS);
    let mut prefix_tokens = Vec::with_capacity(TURNS);
    let mut prefix_hashes = Vec::with_capacity(TURNS);
    let mut assembly_ms = Vec::with_capacity(TURNS);
    let mut recall_ms = Vec::with_capacity(TURNS);
    let mut save_ms = Vec::with_capacity(TURNS);
    let mut per_turn = Vec::with_capacity(TURNS);
    let mut totals: BTreeMap<String, u64> = BTreeMap::new();
    let mut compactions = 0usize;

    for turn in 0..TURNS {
        let user_text = format!(
            "Turn {}: what do you know about my preferences and the project?",
            turn + 1
        );
        let submitted = session
            .submit(
                &conversation,
                &format!("tokens-{turn}"),
                "input",
                json!({"text": user_text}),
            )
            .with_context(|| format!("submit turn {turn}"))?;
        agent
            .run_submission(&submitted.submission.id)
            .with_context(|| format!("run turn {turn}"))?;

        let history = session.history_for_prompt(&conversation)?;
        let standing = memory.standing()?;
        let standing_text = memory.render_standing(&standing.items);
        let recall_started = Instant::now();
        let recalled = memory.recall_read(&user_text)?;
        recall_ms.push(recall_started.elapsed().as_secs_f64() * 1000.0);
        let recall_text = memory.render_recall(&recalled);
        let runtime_note = format!(
            "Current time: 2025-01-01T00:00:00Z. Conversation: {conversation}. Model: {model}."
        );
        let assembly_started = Instant::now();
        let prompt = build_prompt(
            PromptInput {
                soul: soul.clone(),
                conversation_id: conversation.clone(),
                model: model.clone(),
                tools: tool_specs.clone(),
                standing: standing_text,
                standing_version: standing.version,
                recall: recall_text,
                skills_index: String::new(),
                guidance: String::new(),
                runtime_note,
            },
            &history,
        )
        .with_context(|| format!("assemble turn {turn}"))?;
        assembly_ms.push(assembly_started.elapsed().as_secs_f64() * 1000.0);

        fixed.push(prompt.tokens.fixed);
        let prefix = prompt.tokens.identity
            + prompt.tokens.operation
            + prompt.tokens.tools
            + prompt.tokens.standing
            + prompt.tokens.skills;
        prefix_tokens.push(prefix);
        prefix_hashes.push(prompt.prefix_hash.clone());
        for (name, value) in [
            ("identity", prompt.tokens.identity),
            ("operation", prompt.tokens.operation),
            ("tools", prompt.tokens.tools),
            ("standing", prompt.tokens.standing),
            ("skills", prompt.tokens.skills),
            ("transcript", prompt.tokens.transcript),
            ("recall", prompt.tokens.recall),
            ("runtime", prompt.tokens.runtime),
            ("total", prompt.tokens.total),
        ] {
            *totals.entry(name.to_owned()).or_default() += value;
        }

        let expected = if turn == 0 { None } else { Some(turn as u64) };
        let save_started = Instant::now();
        session
            .save(Save::new(&timing).document("timing", expected, json!({"turn": turn})))
            .with_context(|| format!("time save {turn}"))?;
        save_ms.push(save_started.elapsed().as_secs_f64() * 1000.0);

        if agent
            .compact_soft(&conversation)
            .with_context(|| format!("compact after turn {turn}"))?
        {
            compactions += 1;
        }
        let cache_read = provider
            .last_chat()
            .map(|record| record.cache_read_tokens)
            .unwrap_or(0);
        per_turn.push(TurnReport {
            turn: turn + 1,
            total: prompt.tokens.total,
            fixed: prompt.tokens.fixed,
            prefix,
            cache_read,
        });
    }

    let history = session.history(&conversation)?;
    let summary_tokens: u64 = history
        .iter()
        .filter_map(summary_text)
        .map(|text| estimate_tokens(text) as u64)
        .sum();
    let fixed_overhead_p95 = p95_u64(&fixed);
    let fixed_overhead_max = fixed.iter().copied().max().unwrap_or(0);
    let prefix_tokens_p95 = p95_u64(&prefix_tokens);
    let records = provider.chat_records();
    let prefix_stable = prefix_hashes
        .windows(2)
        .all(|window| window[0] == window[1])
        && records
            .windows(2)
            .all(|window| window[0].prefix_hash == window[1].prefix_hash);
    let prompt_tokens_p95 = p95_u64(
        &records
            .iter()
            .map(|record| record.total_tokens)
            .collect::<Vec<_>>(),
    );
    let cache_read_total: u64 = records.iter().map(|record| record.cache_read_tokens).sum();
    let prefix_total: u64 = records.iter().map(|record| record.prefix_tokens).sum();
    let cache_read_share = if prefix_total == 0 {
        0.0
    } else {
        cache_read_total as f64 / prefix_total as f64
    };
    let cache_hit_turns = records
        .iter()
        .filter(|record| record.cache_read_tokens > 0)
        .count();
    let save_ms_p95 = p95_f64(&save_ms);
    let assembly_ms_p95 = p95_f64(&assembly_ms);
    let recall_ms_p95 = p95_f64(&recall_ms);
    let targets = vec![
        TargetReport {
            name: "fixed prompt overhead".to_owned(),
            measured: fixed_overhead_p95 as f64,
            target: TARGET_FIXED as f64,
            unit: "tokens".to_owned(),
            met: fixed_overhead_p95 <= TARGET_FIXED,
        },
        TargetReport {
            name: "prefix cache share".to_owned(),
            measured: cache_read_share,
            target: TARGET_CACHE,
            unit: "share".to_owned(),
            met: cache_read_share >= TARGET_CACHE,
        },
        TargetReport {
            name: "save p95".to_owned(),
            measured: save_ms_p95,
            target: TARGET_SAVE_MS,
            unit: "ms".to_owned(),
            met: save_ms_p95 <= TARGET_SAVE_MS,
        },
        TargetReport {
            name: "prompt assembly p95".to_owned(),
            measured: assembly_ms_p95,
            target: TARGET_ASSEMBLY_MS,
            unit: "ms".to_owned(),
            met: assembly_ms_p95 <= TARGET_ASSEMBLY_MS,
        },
        TargetReport {
            name: "recall p95".to_owned(),
            measured: recall_ms_p95,
            target: TARGET_RECALL_MS,
            unit: "ms".to_owned(),
            met: recall_ms_p95 <= TARGET_RECALL_MS,
        },
    ];
    let deferred = vec![
        "startup and recovery with 100k entries (M8)".to_owned(),
        "idle memory use (M8)".to_owned(),
        "annual database size (M8)".to_owned(),
    ];

    Ok(TokensReport {
        turns: TURNS,
        compactions,
        summary_tokens,
        fixed_overhead_p95,
        fixed_overhead_max,
        prefix_tokens_p95,
        prompt_tokens_p95,
        prefix_stable,
        cache_read_share,
        cache_hit_turns,
        save_ms_p95,
        assembly_ms_p95,
        recall_ms_p95,
        token_totals: totals,
        per_turn,
        targets,
        deferred,
    })
}

#[derive(Debug, Clone)]
struct ChatRecord {
    prefix_hash: String,
    prefix_tokens: u64,
    total_tokens: u64,
    cache_read_tokens: u64,
}

struct EfficiencyProvider {
    chat_records: Mutex<Vec<ChatRecord>>,
    last_chat_prefix: Mutex<Option<String>>,
    chat_count: AtomicUsize,
}

impl EfficiencyProvider {
    fn new() -> Self {
        Self {
            chat_records: Mutex::new(Vec::new()),
            last_chat_prefix: Mutex::new(None),
            chat_count: AtomicUsize::new(0),
        }
    }

    fn chat_records(&self) -> Vec<ChatRecord> {
        self.chat_records
            .lock()
            .map(|records| records.clone())
            .unwrap_or_default()
    }

    fn last_chat(&self) -> Option<ChatRecord> {
        self.chat_records.lock().ok()?.last().cloned()
    }
}

impl Provider for EfficiencyProvider {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        let first = request
            .messages
            .first()
            .map(message_text)
            .unwrap_or_default();
        if first.starts_with("You extract durable memory") {
            let text = "{\"operations\":[]}";
            let usage = usage_for(&request, 0, estimate_tokens(text) as u64);
            return Ok(stream_text(text, usage));
        }
        if first.starts_with("You summarize an older part") {
            let text = "Summary: the user discussed preferences and the project.";
            let usage = usage_for(&request, 0, estimate_tokens(text) as u64);
            return Ok(stream_text(text, usage));
        }

        let standing = request
            .messages
            .iter()
            .skip(1)
            .find(|message| message.role == Role::System)
            .map(message_text)
            .unwrap_or_default();
        let tools = tools_json(&request.tools);
        let prefix = format!("{first}\n{tools}\n{standing}");
        let prefix_hash = hash_text(&prefix);
        let prefix_tokens = estimate_tokens(&prefix) as u64;
        let total_tokens = prompt_tokens(&request);
        let cache_read_tokens = {
            let mut last = self
                .last_chat_prefix
                .lock()
                .map_err(|_| ProviderError::Config("prefix lock poisoned".to_owned()))?;
            let hit = last.as_deref() == Some(prefix_hash.as_str());
            *last = Some(prefix_hash.clone());
            if hit { prefix_tokens } else { 0 }
        };
        if let Ok(mut records) = self.chat_records.lock() {
            records.push(ChatRecord {
                prefix_hash,
                prefix_tokens,
                total_tokens,
                cache_read_tokens,
            });
        }
        let turn = self.chat_count.fetch_add(1, Ordering::SeqCst) + 1;
        let tool_followup = request
            .messages
            .last()
            .is_some_and(|message| message.role == Role::Tool);
        if !tool_followup && turn.is_multiple_of(7) {
            let call = ToolCall {
                id: format!("call-{turn}"),
                name: "files".to_owned(),
                arguments: json!({"action": "read", "path": "notes.txt"}),
            };
            let output = estimate_tokens("files read") as u64;
            let usage = usage_from_totals(total_tokens, cache_read_tokens, output);
            Ok(Box::new(
                vec![
                    Ok(StreamEvent::ToolCall(call)),
                    Ok(StreamEvent::Usage(usage)),
                    Ok(StreamEvent::Done {
                        finish_reason: Some("tool_calls".to_owned()),
                    }),
                ]
                .into_iter(),
            ))
        } else {
            let text = if tool_followup {
                format!("Turn {turn} answer after reading the file.")
            } else {
                format!("Turn {turn} answer. The current project is Hearth.")
            };
            let output = estimate_tokens(&text) as u64;
            let usage = usage_from_totals(total_tokens, cache_read_tokens, output);
            Ok(stream_text(&text, usage))
        }
    }
}

fn stream_text(text: &str, usage: Usage) -> ProviderStream {
    Box::new(
        vec![
            Ok(StreamEvent::TextDelta(text.to_owned())),
            Ok(StreamEvent::Usage(usage)),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ]
        .into_iter(),
    )
}

fn usage_for(request: &ChatRequest, cache_read: u64, output: u64) -> Usage {
    usage_from_totals(prompt_tokens(request), cache_read, output)
}

fn usage_from_totals(input: u64, cache_read: u64, output: u64) -> Usage {
    Usage {
        input_tokens: input as i64,
        cache_read_tokens: cache_read as i64,
        cache_write_tokens: 0,
        output_tokens: output as i64,
        reasoning_tokens: 0,
    }
}

fn prompt_tokens(request: &ChatRequest) -> u64 {
    let messages: u64 = request
        .messages
        .iter()
        .map(|message| estimate_tokens(&message_text(message)) as u64)
        .sum();
    messages + estimate_tokens(&tools_json(&request.tools)) as u64
}

fn message_text(message: &ChatMessage) -> String {
    let mut text = String::new();
    for block in &message.blocks {
        match block {
            ContentBlock::Text(value) | ContentBlock::Thinking(value) => text.push_str(value),
            ContentBlock::ToolCall(call) => {
                text.push_str(&call.name);
                text.push(' ');
                text.push_str(&call.arguments.to_string());
            }
            ContentBlock::ToolResult { content, .. } => text.push_str(content),
            ContentBlock::Image { .. } | ContentBlock::File { .. } => {}
        }
    }
    text
}

fn tools_json(tools: &[ToolSpec]) -> String {
    let values: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect();
    serde_json::to_string(&values).unwrap_or_default()
}

fn hash_text(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn p95_u64(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[percentile_index(sorted.len())]
}

fn p95_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    sorted[percentile_index(sorted.len())]
}

fn percentile_index(len: usize) -> usize {
    let index = ((len as f64) * 0.95).ceil() as usize;
    index.saturating_sub(1).min(len - 1)
}
