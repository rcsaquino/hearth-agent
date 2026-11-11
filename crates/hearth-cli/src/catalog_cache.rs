//! The last models.dev lookup, kept for reset defaults.

use std::fs;
use std::path::{Path, PathBuf};

use hearth_model::ModelLimits;
use serde_json::{Value, json};

const CACHE_FILE: &str = "cache/models.dev.json";

fn cache_path(home: &Path) -> PathBuf {
    home.join(CACHE_FILE)
}

/// The cached models.dev limits for this provider and model.
pub fn load(home: &Path, base_url: &str, model: &str) -> Option<ModelLimits> {
    if home.as_os_str().is_empty() {
        return None;
    }
    let text = fs::read_to_string(cache_path(home)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    if value["base_url"].as_str() != Some(base_url) || value["model"].as_str() != Some(model) {
        return None;
    }
    Some(ModelLimits {
        context_window: value["context_window"].as_u64().unwrap_or(0),
        max_output_tokens: value["max_output_tokens"].as_u64().unwrap_or(0),
    })
}

/// Remember a models.dev lookup for later resets.
pub fn save(home: &Path, base_url: &str, model: &str, limits: ModelLimits) -> Result<(), String> {
    if home.as_os_str().is_empty() {
        return Ok(());
    }
    let path = cache_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(&json!({
        "base_url": base_url,
        "model": model,
        "context_window": limits.context_window,
        "max_output_tokens": limits.max_output_tokens,
    }))
    .map_err(|error| error.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn limits() -> ModelLimits {
        ModelLimits {
            context_window: 1_000_000,
            max_output_tokens: 384_000,
        }
    }

    #[test]
    fn a_saved_lookup_loads_for_the_same_pair() {
        let home = TempDir::new().expect("home");
        save(
            home.path(),
            "https://opencode.ai/inference/go/openai/v1",
            "deepseek-v4.1-flash",
            limits(),
        )
        .expect("save");
        assert_eq!(
            load(
                home.path(),
                "https://opencode.ai/inference/go/openai/v1",
                "deepseek-v4.1-flash"
            ),
            Some(limits())
        );
        assert!(
            load(
                home.path(),
                "https://opencode.ai/inference/go/openai/v1",
                "another-model"
            )
            .is_none()
        );
    }
}
