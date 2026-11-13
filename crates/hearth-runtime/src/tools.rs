//! Shared runtime tool construction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hearth_config::Settings;
use hearth_core::Session;
use hearth_memory::MemoryService;
use hearth_model::{ImageClient, Speaker, SpeechClient, Transcriber, TranscriptionClient};
use hearth_telegram::{MessageTool, TelegramSender};
use hearth_tools::{
    BraveSearch, BrowserTool, DeepSeekFetch, DeepSeekSearch, DocumentsTool, ExaSearch, FetchError,
    FetchProvider, FileTool, FirecrawlFetch, FirecrawlSearch, HttpFetch, JinaFetch, MediaTool,
    SearchError, SearchProvider, ShellTool, TinyFishFetch, TinyFishSearch, Tool, ToolRegistry,
    WebFetchTool, WebSearchTool,
};
use thiserror::Error;

use crate::helpers::{HelperControls, TaskTool};
use crate::scheduler::ScheduleTool;
use crate::skills::{SkillLibrary, SkillTool};
use crate::tool_groups::{ToolGroup, ToolGroups, ToolSearchTool};

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

/// The core tools plus the lazy group registry.
pub struct BuiltTools {
    pub core: ToolRegistry,
    pub groups: Arc<ToolGroups>,
    pub selection: Arc<Mutex<HashSet<String>>>,
}

/// The inbound and outbound media roots used by the optional tools.
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
    workspace: &Path,
    spill: &Path,
    message: Option<MessageTarget>,
    memory: Option<Arc<MemoryService>>,
    skills: Option<Arc<SkillLibrary>>,
    task: Option<TaskTarget>,
    media: &ToolMediaRoots,
) -> Result<BuiltTools, ToolBuildError> {
    let groups = Arc::new(groups_for(settings, workspace, spill, media));
    let selection = Arc::new(Mutex::new(HashSet::new()));
    let mut tools: Vec<Arc<dyn hearth_tools::Tool>> = vec![
        Arc::new(FileTool::new(workspace, spill)),
        Arc::new(ShellTool::new(workspace, Duration::from_secs(60))),
        Arc::new(build_web_search(settings)?),
        Arc::new(build_web_fetch(settings)?),
    ];
    // `tool_search` only earns its tokens when there is a group to reach.
    if groups.any_available() {
        tools.push(Arc::new(ToolSearchTool::new(
            Arc::clone(&groups),
            Arc::clone(&selection),
        )));
    }
    if let Some(memory) = memory {
        tools.extend(memory.tools());
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
    Ok(BuiltTools {
        core: ToolRegistry::new(tools),
        groups,
        selection,
    })
}

fn groups_for(
    settings: &Settings,
    workspace: &Path,
    spill: &Path,
    media: &ToolMediaRoots,
) -> ToolGroups {
    let documents = Arc::new(DocumentsTool::new(
        workspace.to_path_buf(),
        spill.to_path_buf(),
        media.inbound.clone(),
    ));
    let media_tools: Vec<Arc<dyn Tool>> = build_media_tool(settings, workspace, spill, media)
        .map(|tool| vec![Arc::new(tool) as Arc<dyn Tool>])
        .unwrap_or_default();
    let browser_tools: Vec<Arc<dyn Tool>> = BrowserTool::new(
        std::env::var("CAMOFOX_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:9377".to_owned()),
        workspace.to_path_buf(),
        Duration::from_secs(30),
    )
    .map(|tool| vec![Arc::new(tool) as Arc<dyn Tool>])
    .unwrap_or_default();
    ToolGroups::new(vec![
        ToolGroup::new(
            "browser",
            "Browse and interact with web pages that plain fetch cannot reach.",
            browser_tools,
        ),
        ToolGroup::new(
            "documents",
            "Read PDF, DOCX, PPTX, and XLSX files.",
            vec![documents],
        ),
        ToolGroup::new(
            "media",
            "Generate images, transcribe audio, and synthesize speech.",
            media_tools,
        ),
    ])
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
            .filter(|key| !key.is_empty());
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
            .filter(|key| !key.is_empty());
        match name.as_str() {
            "jina" => providers.push(Arc::new(JinaFetch::new("https://r.jina.ai", key)?)),
            "http" | "https" => {
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
            &["jina", "tinyfish", "http", "firecrawl", "https", "deepseek"],
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
}

/// The ceiling for the always-loaded tool schemas.
///
/// Measured at 2,359 tokens: roughly 1,410 for the five memory tools and about
/// 950 for the rest of the core set. The guard exists to catch a single tool
/// growing a sprawling schema, so it sits just above the measured total rather
/// than constraining the memory design.
#[cfg(test)]
const CORE_SCHEMA_TOKEN_BUDGET: i64 = 2_600;

#[cfg(test)]
mod core_budget_tests {
    use super::*;
    use hearth_core::Harness;
    use hearth_memory::MemoryService;
    use hearth_telegram::{SystemSleeper, TelegramClient};
    use serde_json::json;

    fn settings() -> Settings {
        serde_json::from_value(json!({})).expect("empty settings")
    }

    #[test]
    fn the_core_tool_schemas_fit_the_budget() {
        let home = tempfile::TempDir::new().expect("temp home");
        let harness = Harness::open(home.path()).expect("harness");
        let memory = Arc::new(MemoryService::new(harness.session().clone()).expect("memory"));
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
            outbound: home.path().join("outbound"),
            speaker: None,
        };
        let spill = home.path().join("spill");
        let media = ToolMediaRoots {
            inbound: home.path().join("media/inbound"),
            outbound: home.path().join("media/outbound"),
        };
        let built = build_tools(
            &settings(),
            home.path(),
            &spill,
            Some(target),
            Some(memory),
            Some(Arc::new(SkillLibrary::new(home.path()))),
            None,
            &media,
        )
        .expect("tools");

        let names: Vec<String> = built
            .core
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        for expected in [
            "files",
            "shell",
            "web_search",
            "web_fetch",
            "memory_add",
            "memory_edit",
            "memory_delete",
            "memory_search",
            "memory_sessions",
            "message",
            "schedule",
            "skill",
            "tool_search",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected}"
            );
        }

        let tokens = ToolGroups::schema_tokens(&built.core.specs());
        // Memory contributes five tools, about 1,410 tokens of this total. The
        // rest of the core set is roughly 950. The guard exists to catch a tool
        // that grew a sprawling schema, not to forbid the memory family.
        assert!(
            tokens <= CORE_SCHEMA_TOKEN_BUDGET,
            "core tool schemas use {tokens} tokens"
        );
    }

    #[test]
    fn real_documents_and_media_groups_load_lazily() {
        let home = tempfile::TempDir::new().expect("temp home");
        let workspace = home.path().join("workspace");
        let spill = home.path().join("spill");
        let inbound = home.path().join("media/inbound");
        let outbound = home.path().join("media/outbound");
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::create_dir_all(&spill).expect("spill");
        std::fs::create_dir_all(&inbound).expect("inbound");
        std::fs::create_dir_all(&outbound).expect("outbound");
        let settings: Settings = serde_json::from_value(json!({
            "media": {
                "image": {"kind": "openrouter-images", "api_key": "fake", "model": "image-model"},
                "stt": {"kind": "groq", "api_key": "fake", "model": "whisper-large-v3-turbo"},
                "tts": {"kind": "elevenlabs", "api_key": "fake", "model": "eleven_v4", "voice_id": "voice-id"}
            }
        }))
        .expect("settings");
        let media = ToolMediaRoots { inbound, outbound };
        let built = build_tools(
            &settings, &workspace, &spill, None, None, None, None, &media,
        )
        .expect("tools");

        let documents = built.groups.get("documents").expect("documents group");
        assert!(documents.available());
        assert_eq!(documents.specs()[0].name, "documents");
        let media_group = built.groups.get("media").expect("media group");
        assert!(media_group.available());
        assert_eq!(media_group.specs()[0].name, "media");
        let browser = built.groups.get("browser").expect("browser group");
        assert!(browser.available());
        assert_eq!(browser.specs()[0].name, "browser");

        let empty = HashSet::new();
        assert!(built.groups.selected_specs(&empty).is_empty());
        let core_names: Vec<String> = built
            .core
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert!(
            !core_names
                .iter()
                .any(|name| name == "documents" || name == "media" || name == "browser"),
            "{core_names:?}"
        );

        let mut selected = HashSet::new();
        selected.insert("documents".to_owned());
        let specs = built.groups.selected_specs(&selected);
        assert!(specs.iter().any(|spec| spec.name == "documents"));
        selected.insert("browser".to_owned());
        let specs = built.groups.selected_specs(&selected);
        assert!(specs.iter().any(|spec| spec.name == "browser"));
        let outcome =
            built
                .groups
                .call_selected("documents", &json!({"path": "missing.pdf"}), &selected);
        assert!(outcome.is_some(), "the selected documents tool is callable");
        assert!(outcome.unwrap().is_err());
        assert!(ToolGroups::schema_tokens(&built.core.specs()) <= CORE_SCHEMA_TOKEN_BUDGET);
    }
}
