//! Shared runtime tool construction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hearth_config::Settings;
use hearth_memory::{MemoryPolicy, MemoryService, MemorySource, MemoryTool};
use hearth_model::{Price, Pricing, Speaker};
use hearth_telegram::{MessageTool, TelegramSender};
use hearth_tools::{
    BraveSearch, DeepSeekFetch, DeepSeekSearch, ExaSearch, FetchError, FetchProvider, FileTool,
    FirecrawlFetch, FirecrawlSearch, HttpFetch, JinaFetch, SearchError, SearchProvider, ShellTool,
    TinyFishFetch, TinyFishSearch, ToolRegistry, WebFetchTool, WebSearchTool,
};
use thiserror::Error;

use crate::tool_groups::{ToolGroups, ToolSearchTool};

/// The Telegram values needed for the message tool.
pub struct MessageTarget {
    pub sender: TelegramSender,
    pub chat_id: i64,
    pub thread_id: Option<i64>,
    pub outbound: PathBuf,
    pub speaker: Option<Arc<dyn Speaker>>,
}

/// The core tools plus the lazy group registry.
pub struct BuiltTools {
    pub core: ToolRegistry,
    pub groups: Arc<ToolGroups>,
    pub selection: Arc<Mutex<HashSet<String>>>,
}

/// A tool could not be built from settings.
#[derive(Debug, Error)]
pub enum ToolBuildError {
    #[error(transparent)]
    Search(#[from] SearchError),
    #[error(transparent)]
    Fetch(#[from] FetchError),
}

/// Read per-model prices from settings. Unknown models stay unpriced.
pub fn pricing_from_settings(settings: &Settings) -> Pricing {
    let mut pricing = Pricing::new();
    if let Some(models) = &settings.models {
        for (model, price) in &models.pricing {
            pricing.insert(
                model.clone(),
                Price {
                    input: price.input,
                    cache_read: price.cache_read,
                    cache_write: price.cache_write,
                    output: price.output,
                    reasoning: price.reasoning,
                },
            );
        }
    }
    pricing
}

/// Read the memory budgets from settings.
pub fn memory_policy(settings: &Settings) -> MemoryPolicy {
    MemoryPolicy {
        standing_max_tokens: settings.memory.standing_max_tokens as usize,
        recall_max_tokens: settings.memory.recall_max_tokens as usize,
        recall_max_items: MemoryPolicy::default().recall_max_items,
    }
}

/// Build the tools for one run.
pub fn build_tools(
    settings: &Settings,
    workspace: &Path,
    spill: &Path,
    message: Option<MessageTarget>,
    memory: Option<Arc<MemoryService>>,
    memory_source: Option<MemorySource>,
) -> Result<BuiltTools, ToolBuildError> {
    let groups = Arc::new(ToolGroups::default_groups());
    let selection = Arc::new(Mutex::new(HashSet::new()));
    let mut tools: Vec<Arc<dyn hearth_tools::Tool>> = vec![
        Arc::new(FileTool::new(workspace, spill)),
        Arc::new(ShellTool::new(workspace, Duration::from_secs(60))),
        Arc::new(build_web_search(settings)?),
        Arc::new(build_web_fetch(settings)?),
        Arc::new(ToolSearchTool::new(
            Arc::clone(&groups),
            Arc::clone(&selection),
        )),
    ];
    if let Some(memory) = memory {
        let tool = match memory_source {
            Some(source) => MemoryTool::with_source(memory, source),
            None => MemoryTool::new(memory),
        };
        tools.push(Arc::new(tool));
    }
    if let Some(target) = message {
        tools.push(Arc::new(MessageTool::new(
            target.sender,
            target.chat_id,
            target.thread_id,
            workspace,
            spill,
            target.outbound,
            target.speaker,
        )));
    }
    Ok(BuiltTools {
        core: ToolRegistry::new(tools),
        groups,
        selection,
    })
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
        let memory = Arc::new(MemoryService::new(harness.session().clone()));
        let sender = TelegramSender::new(
            TelegramClient::with_base_url("test-token", "http://127.0.0.1:1")
                .expect("telegram client"),
            Arc::new(SystemSleeper),
        );
        let target = MessageTarget {
            sender,
            chat_id: 1,
            thread_id: None,
            outbound: home.path().join("outbound"),
            speaker: None,
        };
        let spill = home.path().join("spill");
        let built = build_tools(
            &settings(),
            home.path(),
            &spill,
            Some(target),
            Some(memory),
            None,
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
            "memory",
            "message",
            "tool_search",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected}"
            );
        }

        let tokens = ToolGroups::schema_tokens(&built.core.specs());
        assert!(tokens <= 1200, "core tool schemas use {tokens} tokens");
    }
}
