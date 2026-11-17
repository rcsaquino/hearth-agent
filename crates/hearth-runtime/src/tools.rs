//! Shared runtime tool construction.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hearth_config::Settings;
use hearth_core::Session;
use hearth_memory::MemoryService;
use hearth_model::{ImageClient, Speaker, SpeechClient, Transcriber, TranscriptionClient};
use hearth_telegram::{MessageTool, TelegramSender};
use hearth_tools::{
    BraveSearch, DeepSeekFetch, DeepSeekSearch, ExaSearch, FetchError, FetchProvider, FileTool,
    FirecrawlFetch, FirecrawlSearch, HttpFetch, JinaFetch, MediaTool, SearchError, SearchProvider,
    ShellTool, TinyFishFetch, TinyFishSearch, Tool, ToolRegistry, WebFetchTool, WebSearchTool,
};
use thiserror::Error;

use crate::helpers::{HelperControls, TaskTool};
use crate::scheduler::ScheduleTool;
use crate::skills::{SkillLibrary, SkillTool};

/// The Telegram values needed for the message tool.
pub struct MessageTarget {
    pub session: Session,
    pub conversation_id: String,
    pub sender: TelegramSender,
    pub chat_id: i64,
    pub thread_id: Option<i64>,
    pub outbound: PathBuf,
    pub speaker: Option<Arc<dyn Speaker>>,
}

/// Helper spawning values attached to one tool registry.
pub struct TaskTarget {
    pub depth: u8,
    pub controls: Arc<HelperControls>,
    pub max_depth: u8,
    pub max_concurrent: u8,
}

/// The inbound and outbound media roots used by the media tool.
#[derive(Clone, Debug)]
pub struct ToolMediaRoots {
    pub inbound: PathBuf,
    pub outbound: PathBuf,
}

/// A tool could not be built from settings.
#[derive(Debug, Error)]
pub enum ToolBuildError {
    #[error(transparent)]
    Search(#[from] SearchError),
    #[error(transparent)]
    Fetch(#[from] FetchError),
}

/// Build the tools for one run.
#[allow(clippy::too_many_arguments)]
pub fn build_tools(
    settings: &Settings,
    zone: hearth_core::Zone,
    workspace: &Path,
    spill: &Path,
    message: Option<MessageTarget>,
    memory: Option<Arc<MemoryService>>,
    memory_source: Option<String>,
    skills: Option<Arc<SkillLibrary>>,
    task: Option<TaskTarget>,
    media: &ToolMediaRoots,
) -> Result<ToolRegistry, ToolBuildError> {
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(FileTool::new(workspace, spill)),
        Arc::new(ShellTool::new(workspace, Duration::from_secs(60))),
        Arc::new(build_web_search(settings)?),
        Arc::new(build_web_fetch(settings)?),
    ];
    if let Some(media_tool) = build_media_tool(settings, workspace, spill, media) {
        tools.push(Arc::new(media_tool));
    }
    if let Some(memory) = memory {
        tools.extend(memory.tools_at(memory_source, zone));
    }
    if let Some(skills) = skills {
        tools.push(Arc::new(SkillTool::new(skills)));
    }
    if let Some(target) = message {
        tools.push(Arc::new(MessageTool::new(
            target.sender.clone(),
            target.chat_id,
            target.thread_id,
            workspace,
            spill,
            target.outbound,
            target.speaker,
        )));
        tools.push(Arc::new(ScheduleTool::new(
            target.session.clone(),
            target.conversation_id.clone(),
            target.chat_id,
            target.thread_id,
        )));
        if let Some(task) = task
            && task.depth < task.max_depth
        {
            tools.push(Arc::new(TaskTool::new(
                target.session.clone(),
                target.conversation_id.clone(),
                target.chat_id,
                target.thread_id,
                task.depth,
                task.controls,
                task.max_depth,
                task.max_concurrent,
            )));
        }
    }
    Ok(ToolRegistry::new(tools))
}

fn build_media_tool(
    settings: &Settings,
    workspace: &Path,
    spill: &Path,
    media: &ToolMediaRoots,
) -> Option<MediaTool> {
    let config = settings.media.as_ref()?;
    let image = config.image.as_ref().and_then(|service| {
        let api_key = service.api_key.clone().filter(|value| !value.is_empty())?;
        let model = service.model.clone().filter(|value| !value.is_empty())?;
        let base_url = service
            .base_url
            .clone()
            .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_owned());
        ImageClient::new(base_url, api_key, model).ok()
    });
    let transcriber = config.stt.as_ref().and_then(|service| {
        let api_key = service.api_key.clone().filter(|value| !value.is_empty())?;
        let model = service.model.clone().filter(|value| !value.is_empty())?;
        let base_url = service
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.groq.com".to_owned());
        TranscriptionClient::new(base_url, api_key, model)
            .ok()
            .map(|client| Arc::new(client) as Arc<dyn Transcriber>)
    });
    let speaker = config.tts.as_ref().and_then(|service| {
        let api_key = service.api_key.clone().filter(|value| !value.is_empty())?;
        let voice_id = service.voice_id.clone().filter(|value| !value.is_empty())?;
        let model = service
            .model
            .clone()
            .unwrap_or_else(|| "eleven_v4".to_owned());
        let output_format = service
            .output_format
            .clone()
            .unwrap_or_else(|| "opus_48000_64".to_owned());
        let base_url = service
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.elevenlabs.io".to_owned());
        SpeechClient::new(base_url, api_key, voice_id, model, output_format)
            .ok()
            .map(|client| Arc::new(client) as Arc<dyn Speaker>)
    });
    if image.is_none() && transcriber.is_none() && speaker.is_none() {
        return None;
    }
    Some(MediaTool::new(
        image,
        transcriber,
        speaker,
        workspace.to_path_buf(),
        spill.to_path_buf(),
        media.inbound.clone(),
        media.outbound.clone(),
    ))
}

fn build_web_search(settings: &Settings) -> Result<WebSearchTool, SearchError> {
    let search = settings
        .web
        .as_ref()
        .map(|web| web.search.clone())
        .unwrap_or_default();
    let mut providers: Vec<Arc<dyn SearchProvider>> = Vec::new();
    for name in &search.order {
        let key = search
            .providers
            .get(name)
            .and_then(|provider| provider.api_key.clone())
            .filter(|key| hearth_config::is_resolved(key));
        match name.as_str() {
            "exa" => providers.push(Arc::new(ExaSearch::new(
                "https://mcp.exa.ai",
                "https://api.exa.ai",
                key,
            )?)),
            "brave" => {
                if let Some(key) = key {
                    providers.push(Arc::new(BraveSearch::new(
                        "https://api.search.brave.com/res/v1",
                        key,
                    )?));
                }
            }
            "deepseek" => {
                if let Some(key) = key {
                    providers.push(Arc::new(DeepSeekSearch::new(
                        "https://api.deepseek.com",
                        key,
                    )?));
                }
            }
            "firecrawl" => {
                if let Some(key) = key {
                    providers.push(Arc::new(FirecrawlSearch::new(
                        "https://api.firecrawl.dev",
                        key,
                    )?));
                }
            }
            "tinyfish" => {
                if let Some(key) = key {
                    providers.push(Arc::new(TinyFishSearch::new(
                        "https://api.search.tinyfish.ai",
                        key,
                    )?));
                }
            }
            _ => {}
        }
    }
    Ok(WebSearchTool::new(
        providers,
        search.parallel as usize,
        search.results as usize,
        search.max_results as usize,
    ))
}

fn build_web_fetch(settings: &Settings) -> Result<WebFetchTool, FetchError> {
    let fetch = settings
        .web
        .as_ref()
        .map(|web| web.fetch.clone())
        .unwrap_or_default();
    Ok(WebFetchTool::new(build_fetch_providers(&fetch)?, false))
}

fn build_fetch_providers(
    fetch: &hearth_config::WebFetch,
) -> Result<Vec<Arc<dyn FetchProvider>>, FetchError> {
    let mut providers: Vec<Arc<dyn FetchProvider>> = Vec::new();
    let mut direct_added = false;
    for name in &fetch.order {
        let key = fetch
            .providers
            .get(name)
            .and_then(|provider| provider.api_key.clone())
            .filter(|key| hearth_config::is_resolved(key));
        match name.as_str() {
            "jina" => providers.push(Arc::new(JinaFetch::new("https://r.jina.ai", key)?)),
            "http" => {
                if !direct_added {
                    providers.push(Arc::new(HttpFetch::production("http")?));
                    direct_added = true;
                }
            }
            "tinyfish" => {
                if let Some(key) = key {
                    providers.push(Arc::new(TinyFishFetch::production(key)?));
                }
            }
            "firecrawl" => {
                if let Some(key) = key {
                    providers.push(Arc::new(FirecrawlFetch::production(key)?));
                }
            }
            "deepseek" => {
                if let Some(key) = key {
                    providers.push(Arc::new(DeepSeekFetch::production(key)?));
                }
            }
            _ => {}
        }
    }
    Ok(providers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearth_config::{FetchProvider as FetchProviderConfig, WebFetch};
    use std::collections::BTreeMap;

    fn configured(order: &[&str], keys: &[(&str, &str)]) -> WebFetch {
        let mut providers = BTreeMap::new();
        for (name, key) in keys {
            providers.insert(
                (*name).to_owned(),
                FetchProviderConfig {
                    api_key: Some((*key).to_owned()),
                },
            );
        }
        WebFetch {
            order: order.iter().map(|name| (*name).to_owned()).collect(),
            providers,
        }
    }

    #[test]
    fn the_fetch_chain_follows_the_order_and_keeps_direct_once() {
        let fetch = configured(
            &["jina", "tinyfish", "http", "firecrawl", "deepseek"],
            &[("tinyfish", "t"), ("firecrawl", "f"), ("deepseek", "d")],
        );
        let chain = build_fetch_providers(&fetch).expect("fetch chain");
        let names: Vec<&str> = chain.iter().map(|provider| provider.name()).collect();
        assert_eq!(
            names,
            vec!["jina", "tinyfish", "http", "firecrawl", "deepseek"]
        );
    }

    #[test]
    fn a_keyed_provider_without_a_key_is_skipped() {
        let fetch = configured(&["tinyfish", "http"], &[]);
        let chain = build_fetch_providers(&fetch).expect("fetch chain");
        let names: Vec<&str> = chain.iter().map(|provider| provider.name()).collect();
        assert_eq!(names, vec!["http"]);
    }

    #[test]
    fn an_unresolved_environment_key_does_not_join_a_chain() {
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "web": {
                "search": {
                    "order": ["exa", "brave"],
                    "providers": {"brave": {"api_key": "${HEARTH_TEST_UNRESOLVED_KEY}"}}
                },
                "fetch": {
                    "order": ["tinyfish", "http"],
                    "providers": {"tinyfish": {"api_key": "${HEARTH_TEST_UNRESOLVED_KEY}"}}
                }
            }
        }))
        .expect("settings");

        let search = build_web_search(&settings).expect("search chain");
        assert_eq!(search.provider_names(), vec!["exa"]);

        let fetch = &settings.web.as_ref().expect("web block").fetch;
        let chain = build_fetch_providers(fetch).expect("fetch chain");
        let names: Vec<&str> = chain.iter().map(|provider| provider.name()).collect();
        assert_eq!(names, vec!["http"]);
    }
}

/// The ceiling for the always-loaded tool schemas.
///
/// One memory tool replaced four. The shared subject, kind, and limit fields and
/// one envelope took the memory family from 811 to 491 tokens. The registry
/// measures 1,294 tokens, and 1,459 with image, speech to text, and text to
/// speech all configured, so the guard covers the widest run rather than the
/// smallest. It sits just above the measured total so a single tool growing a
/// sprawling schema is caught.
#[cfg(test)]
const CORE_SCHEMA_TOKEN_BUDGET: i64 = 1_470;

/// The memory tool's share of the core schema budget.
///
/// The tool measures 491 tokens. The guard sits just above the measured total so
/// one action growing a sprawling schema is caught.
#[cfg(test)]
const MEMORY_SCHEMA_TOKEN_BUDGET: i64 = 500;

#[cfg(test)]
mod core_budget_tests {
    use super::*;
    use hearth_core::Harness;
    use hearth_memory::MemoryService;
    use hearth_model::ToolSpec;
    use hearth_telegram::{SystemSleeper, TelegramClient};
    use serde_json::{Value, json};

    fn settings() -> Settings {
        serde_json::from_value(json!({})).expect("empty settings")
    }

    fn schema_tokens(specs: &[ToolSpec]) -> i64 {
        let values: Vec<Value> = specs
            .iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": spec.parameters,
                })
            })
            .collect();
        hearth_model::estimate_tokens(&serde_json::to_string(&values).unwrap_or_default())
    }

    /// The widest core registry a run can carry, so the measured budget covers
    /// every always-loaded schema rather than the smallest set.
    fn full_core_tools(home: &Path, settings: &Settings) -> ToolRegistry {
        let harness = Harness::open(home).expect("harness");
        let memory = Arc::new(MemoryService::new(harness.session().clone()));
        let sender = TelegramSender::new(
            TelegramClient::with_base_url("test-token", "http://127.0.0.1:1")
                .expect("telegram client"),
            Arc::new(SystemSleeper),
        );
        let target = MessageTarget {
            session: harness.session().clone(),
            conversation_id: "test".to_owned(),
            sender,
            chat_id: 1,
            thread_id: None,
            outbound: home.join("outbound"),
            speaker: None,
        };
        let spill = home.join("spill");
        let media = ToolMediaRoots {
            inbound: home.join("media/inbound"),
            outbound: home.join("media/outbound"),
        };
        build_tools(
            settings,
            hearth_core::Zone::utc(),
            home,
            &spill,
            Some(target),
            Some(memory),
            None,
            Some(Arc::new(SkillLibrary::new(home))),
            None,
            &media,
        )
        .expect("tools")
    }

    #[test]
    fn the_core_tool_schemas_fit_the_budget() {
        let home = tempfile::TempDir::new().expect("temp home");
        let tools = full_core_tools(home.path(), &settings());

        let names: Vec<String> = tools.specs().into_iter().map(|spec| spec.name).collect();
        for expected in [
            "files",
            "shell",
            "web_search",
            "web_fetch",
            "memory",
            "message",
            "schedule",
            "skill",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected}"
            );
        }

        let tokens = schema_tokens(&tools.specs());
        assert!(
            tokens <= CORE_SCHEMA_TOKEN_BUDGET,
            "core tool schemas use {tokens} tokens"
        );
        let memory_tokens: i64 = tools
            .specs()
            .iter()
            .filter(|spec| spec.name == "memory")
            .map(|spec| schema_tokens(std::slice::from_ref(spec)))
            .sum();
        assert!(
            memory_tokens <= MEMORY_SCHEMA_TOKEN_BUDGET,
            "the memory schema uses {memory_tokens} tokens"
        );
    }

    /// Any configured media service adds the media schema to the core request,
    /// so the widest registry is measured against the same budget.
    #[test]
    fn the_core_schemas_fit_the_budget_with_media() {
        let home = tempfile::TempDir::new().expect("temp home");
        let settings: Settings = serde_json::from_value(json!({
            "media": {
                "image": {"api_key": "test-key", "model": "test-model"},
                "stt": {"api_key": "test-key", "model": "test-model"},
                "tts": {"api_key": "test-key", "model": "test-model", "voice_id": "test-voice"}
            }
        }))
        .expect("settings");
        let tools = full_core_tools(home.path(), &settings);

        let names: Vec<String> = tools.specs().into_iter().map(|spec| spec.name).collect();
        assert!(names.iter().any(|name| name == "media"), "missing media");

        let tokens = schema_tokens(&tools.specs());
        assert!(
            tokens <= CORE_SCHEMA_TOKEN_BUDGET,
            "core tool schemas with media use {tokens} tokens"
        );
    }
}
