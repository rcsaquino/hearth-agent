//! A minimal line-based local conversation.

use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hearth_config::ConfigError;
use hearth_core::{CoreError, Harness};
use hearth_model::{OpenAiCompatibleProvider, Pricing, ProviderError};
use hearth_tools::{
    BraveSearch, DeepSeekSearch, ExaSearch, FetchError, FetchProvider, FileTool, FirecrawlSearch,
    HttpFetch, JinaFetch, SearchError, SearchProvider, ShellTool, TinyFishSearch, ToolRegistry,
    WebFetchTool, WebSearchTool,
};
use serde_json::json;
use thiserror::Error;
use time::OffsetDateTime;

use crate::agent::{Agent, AgentConfig};

/// A failure that stopped local chat.
#[derive(Debug, Error)]
pub enum ChatError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Search(#[from] SearchError),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("chat input failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Run a line-based conversation in the terminal.
pub fn chat(home: &Path) -> Result<(), ChatError> {
    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;
    let models = settings
        .models
        .as_ref()
        .ok_or_else(|| ConfigError::Invalid("models is required".to_owned()))?;

    let provider = OpenAiCompatibleProvider::new(
        models.base_url.clone(),
        models.api_key.clone(),
        models
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        models.session_header.clone(),
    )?;
    let provider: Arc<dyn hearth_model::Provider> = Arc::new(provider);

    let soul_path = match &settings.identity.soul_file {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => home.join(path),
        None => home.join("SOUL.md"),
    };
    let soul = std::fs::read_to_string(&soul_path).unwrap_or_else(|_| {
        hearth_core::default_soul(&settings.identity.agent_name, &settings.identity.user_name)
    });

    let workspace = match settings.agent.workspace.clone() {
        Some(path) if path.is_absolute() => path,
        Some(path) => home.join(path),
        None => home.join("workspace"),
    };
    let spill = home.join("spill");
    let web_search = Arc::new(build_web_search(&settings)?);
    let web_fetch = Arc::new(build_web_fetch(&settings)?);
    let tools = ToolRegistry::new(vec![
        Arc::new(FileTool::new(workspace.clone(), spill.clone())),
        Arc::new(ShellTool::new(workspace, Duration::from_secs(60))),
        web_search,
        web_fetch,
    ]);

    let harness = Harness::open(home)?;
    let session = harness.session().clone();
    let transport_key = format!("local:{}", OffsetDateTime::now_utc().unix_timestamp_nanos());
    let conversation = session.create_conversation("Chat", "local", transport_key.as_str())?;
    let agent = Agent::new(
        session.clone(),
        provider,
        Pricing::default(),
        AgentConfig {
            soul,
            model: models.model.clone(),
            provider_name: models.kind.clone(),
            owner: format!("chat:{}", std::process::id()),
            max_turns: settings.agent.max_turns,
            tools,
            spill_dir: spill,
        },
    );

    let stdin = std::io::stdin();
    let mut count: u64 = 0;
    for line in stdin.lock().lines() {
        let text = line?;
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        count += 1;
        let request_id = format!("local:{}:{}", conversation.id, count);
        let submitted = session.submit(
            &conversation.id,
            &request_id,
            "input",
            json!({"text": text}),
        )?;
        match agent.run_submission(&submitted.submission.id) {
            Ok(outcome) => {
                if let Some(answer) = outcome.answer_text {
                    println!("{answer}");
                } else if let Some(notice) = outcome.notice {
                    println!("{notice}");
                }
            }
            Err(error) => println!("error: {error}"),
        }
    }
    Ok(())
}

fn build_web_search(settings: &hearth_config::Settings) -> Result<WebSearchTool, SearchError> {
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

fn build_web_fetch(settings: &hearth_config::Settings) -> Result<WebFetchTool, FetchError> {
    let fetch = settings
        .web
        .as_ref()
        .map(|web| web.fetch.clone())
        .unwrap_or_default();
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
            // These providers arrive in M2c-2b. The settings accept them now.
            "tinyfish" | "firecrawl" | "deepseek" => {}
            _ => {}
        }
    }
    Ok(WebFetchTool::new(providers, false))
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
