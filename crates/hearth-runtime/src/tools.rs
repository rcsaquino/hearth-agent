//! Shared runtime tool construction.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hearth_config::Settings;
use hearth_memory::{MemoryPolicy, MemoryService, MemorySource, MemoryTool};
use hearth_model::Speaker;
use hearth_telegram::{MessageTool, TelegramSender};
use hearth_tools::{
    BraveSearch, DeepSeekFetch, DeepSeekSearch, ExaSearch, FetchError, FetchProvider, FileTool,
    FirecrawlFetch, FirecrawlSearch, HttpFetch, JinaFetch, SearchError, SearchProvider, ShellTool,
    TinyFishFetch, TinyFishSearch, ToolRegistry, WebFetchTool, WebSearchTool,
};
use thiserror::Error;

/// The Telegram values needed for the message tool.
pub struct MessageTarget {
    pub sender: TelegramSender,
    pub chat_id: i64,
    pub thread_id: Option<i64>,
    pub outbound: PathBuf,
    pub speaker: Option<Arc<dyn Speaker>>,
}

/// A tool could not be built from settings.
#[derive(Debug, Error)]
pub enum ToolBuildError {
    #[error(transparent)]
    Search(#[from] SearchError),
    #[error(transparent)]
    Fetch(#[from] FetchError),
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
) -> Result<ToolRegistry, ToolBuildError> {
    let mut tools: Vec<Arc<dyn hearth_tools::Tool>> = vec![
        Arc::new(FileTool::new(workspace, spill)),
        Arc::new(ShellTool::new(workspace, Duration::from_secs(60))),
        Arc::new(build_web_search(settings)?),
        Arc::new(build_web_fetch(settings)?),
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
    Ok(ToolRegistry::new(tools))
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
