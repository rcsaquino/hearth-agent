//! Model limits from models.dev.

use std::time::Duration;

use serde_json::Value;

/// The public models.dev catalog.
pub const CATALOG_URL: &str = "https://models.dev/api.json";

const CATALOG_TIMEOUT: Duration = Duration::from_secs(20);

/// The facts Hearth uses from one provider's model entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelLimits {
    pub context_window: u64,
    pub max_output_tokens: u64,
}

/// A failure while fetching or parsing the catalog.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("could not fetch the model catalog: {0}")]
    Transport(String),
    #[error("the model catalog is not valid: {0}")]
    Parse(String),
}

#[derive(Debug, Clone)]
struct Entry {
    api: Option<String>,
    model: String,
    limits: ModelLimits,
}

/// A parsed models.dev catalog, flattened to provider entries.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: Vec<Entry>,
}

impl Catalog {
    /// Parse the models.dev `api.json` shape.
    pub fn parse(text: &str) -> Result<Self, CatalogError> {
        let root: Value =
            serde_json::from_str(text).map_err(|error| CatalogError::Parse(error.to_string()))?;
        let providers = root
            .as_object()
            .ok_or_else(|| CatalogError::Parse("the catalog is not an object".to_owned()))?;
        let mut entries = Vec::new();
        for provider in providers.values() {
            let provider_api = provider.get("api").and_then(Value::as_str);
            let Some(models) = provider.get("models").and_then(Value::as_object) else {
                continue;
            };
            for (model, entry) in models {
                let api = entry
                    .get("provider")
                    .and_then(|provider| provider.get("api"))
                    .and_then(Value::as_str)
                    .or(provider_api);
                let limit = entry.get("limit");
                let context_window = limit
                    .and_then(|limit| limit.get("context"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let max_output_tokens = limit
                    .and_then(|limit| limit.get("output"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if context_window == 0 && max_output_tokens == 0 {
                    continue;
                }
                entries.push(Entry {
                    api: api.map(str::to_owned),
                    model: model.clone(),
                    limits: ModelLimits {
                        context_window,
                        max_output_tokens,
                    },
                });
            }
        }
        Ok(Self { entries })
    }

    /// The matching entry for a base URL and model id.
    pub fn lookup(&self, base_url: &str, model: &str) -> Option<ModelLimits> {
        let wanted = normalize_url(base_url);
        self.entries
            .iter()
            .find(|entry| {
                entry.model.eq_ignore_ascii_case(model)
                    && entry
                        .api
                        .as_deref()
                        .is_some_and(|api| normalize_url(api) == wanted)
            })
            .map(|entry| entry.limits)
    }

    /// The number of parsed provider-model entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Fetch and parse the catalog at `url`.
pub fn fetch(url: &str) -> Result<Catalog, CatalogError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(CATALOG_TIMEOUT)
        .user_agent("Hearth/0.1")
        .build()
        .map_err(|error| CatalogError::Transport(error.to_string()))?;
    let response = client
        .get(url)
        .send()
        .map_err(|error| CatalogError::Transport(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(CatalogError::Transport(format!(
            "the catalog returned {status}"
        )));
    }
    let text = response
        .text()
        .map_err(|error| CatalogError::Transport(error.to_string()))?;
    Catalog::parse(&text)
}

/// Look up a model at the public models.dev catalog.
pub fn lookup(base_url: &str, model: &str) -> Result<Option<ModelLimits>, CatalogError> {
    fetch(CATALOG_URL).map(|catalog| catalog.lookup(base_url, model))
}

fn normalize_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix("/v1").unwrap_or(url);
    url.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::parse(
            r#"{
                "opencode-go": {
                    "api": "https://opencode.ai/inference/go/openai/v1",
                    "models": {
                        "deepseek-v4.1-flash": {
                            "limit": {"context": 1000000, "output": 384000}
                        }
                    }
                },
                "azure": {
                    "models": {
                        "deepseek-v4.1-flash": {
                            "provider": {"api": "https://example.azure.com/v1"},
                            "limit": {"context": 1000000, "output": 128000}
                        }
                    }
                }
            }"#,
        )
        .expect("catalog")
    }

    #[test]
    fn looks_up_the_matching_provider_not_just_the_model_id() {
        let catalog = catalog();
        let info = catalog
            .lookup(
                "https://opencode.ai/inference/go/openai/v1/",
                "deepseek-v4.1-flash",
            )
            .expect("entry");
        assert_eq!(info.context_window, 1_000_000);
        assert_eq!(info.max_output_tokens, 384_000);
        assert_eq!(
            catalog
                .lookup("https://example.azure.com", "deepseek-v4.1-flash")
                .expect("azure")
                .max_output_tokens,
            128_000
        );
    }

    #[test]
    fn an_unknown_pair_has_no_entry() {
        assert!(
            catalog()
                .lookup("https://example.com/v1", "deepseek-v4.1-flash")
                .is_none()
        );
    }
}
