use crate::settings::{
    ImageService, Media, Models, SttService, Tasks, Telegram, TtsService, WebFetch, WebSearch,
};
use crate::{ConfigError, Settings, unresolved};

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
}

fn require(field: &str, value: &str) -> Result<(), ConfigError> {
    if let Some(name) = unresolved(value) {
        return Err(invalid(format!(
            "environment variable {name} is not set (referenced by {field})"
        )));
    }
    if value.is_empty() {
        return Err(invalid(format!("{field} must not be empty")));
    }
    Ok(())
}

fn required(field: &str, block: &str, value: Option<&String>) -> Result<(), ConfigError> {
    match value {
        Some(value) => require(field, value),
        None => Err(invalid(format!(
            "{field} is required when {block} is enabled"
        ))),
    }
}

fn positive(field: &str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(invalid(format!("{field} must be greater than zero")));
    }
    Ok(())
}

fn require_https(field: &str, value: &str) -> Result<(), ConfigError> {
    require(field, value)?;
    if !value.starts_with("https://") {
        return Err(invalid(format!("{field} must use https")));
    }
    Ok(())
}

impl Settings {
    /// Validate the shape of a parsed settings file.
    pub fn validate(&self) -> Result<(), ConfigError> {
        require("identity.agent_name", &self.identity.agent_name)?;
        require("identity.user_name", &self.identity.user_name)?;
        require("identity.timezone", &self.identity.timezone)?;
        if let Some(path) = &self.identity.soul_file {
            require("identity.soul_file", &path.to_string_lossy())?;
        }

        if let Some(telegram) = &self.telegram {
            telegram.validate()?;
        }
        if let Some(models) = &self.models {
            models.validate()?;
        }
        if let Some(media) = &self.media {
            media.validate()?;
        }
        if let Some(web) = &self.web {
            web.search.validate()?;
            web.fetch.validate()?;
        }
        self.tasks.validate()?;

        if let Some(path) = &self.agent.workspace {
            require("agent.workspace", &path.to_string_lossy())?;
        }
        positive("agent.max_turns", u64::from(self.agent.max_turns))?;
        positive("compaction.idle_minutes", self.compaction.idle_minutes)?;
        positive(
            "compaction.idle_min_tokens",
            self.compaction.idle_min_tokens,
        )?;
        positive(
            "memory.standing_max_tokens",
            self.memory.standing_max_tokens,
        )?;
        positive("memory.recall_max_tokens", self.memory.recall_max_tokens)?;

        if !matches!(
            self.logging.level.as_str(),
            "trace" | "debug" | "info" | "warn" | "error"
        ) {
            return Err(invalid(
                "logging.level must be one of trace, debug, info, warn, or error",
            ));
        }
        Ok(())
    }

    /// Validate that the settings can run a conversation.
    pub fn validate_for_run(&self) -> Result<(), ConfigError> {
        self.validate()?;
        if self.models.is_none() {
            return Err(invalid(
                "models is not configured; add the models block or run hearth init",
            ));
        }
        Ok(())
    }
}

impl Telegram {
    fn validate(&self) -> Result<(), ConfigError> {
        let Some(token) = &self.bot_token else {
            return Ok(());
        };
        require("telegram.bot_token", token)?;
        if self.allowed_user_ids.is_empty() {
            return Err(invalid(
                "telegram.allowed_user_ids must not be empty when telegram.bot_token is set",
            ));
        }
        Ok(())
    }
}

impl Media {
    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(image) = &self.image {
            image.validate()?;
        }
        if let Some(stt) = &self.stt {
            stt.validate()?;
        }
        if let Some(tts) = &self.tts {
            tts.validate()?;
        }
        Ok(())
    }
}

impl ImageService {
    fn validate(&self) -> Result<(), ConfigError> {
        required("media.image.kind", "media.image", self.kind.as_ref())?;
        if self.kind.as_deref() != Some("openrouter-images") {
            return Err(invalid("media.image.kind must be `openrouter-images`"));
        }
        required("media.image.api_key", "media.image", self.api_key.as_ref())?;
        required("media.image.model", "media.image", self.model.as_ref())?;
        if let Some(base_url) = &self.base_url {
            require_https("media.image.base_url", base_url)?;
        }
        Ok(())
    }
}

impl TtsService {
    fn validate(&self) -> Result<(), ConfigError> {
        required("media.tts.kind", "media.tts", self.kind.as_ref())?;
        if self.kind.as_deref() != Some("elevenlabs") {
            return Err(invalid("media.tts.kind must be `elevenlabs`"));
        }
        required("media.tts.api_key", "media.tts", self.api_key.as_ref())?;
        required("media.tts.model", "media.tts", self.model.as_ref())?;
        required("media.tts.voice_id", "media.tts", self.voice_id.as_ref())?;
        if let Some(base_url) = &self.base_url {
            require_https("media.tts.base_url", base_url)?;
        }
        if let Some(output_format) = &self.output_format {
            require("media.tts.output_format", output_format)?;
        }
        Ok(())
    }
}

impl SttService {
    fn validate(&self) -> Result<(), ConfigError> {
        required("media.stt.kind", "media.stt", self.kind.as_ref())?;
        if self.kind.as_deref() != Some("groq") {
            return Err(invalid("media.stt.kind must be `groq`"));
        }
        required("media.stt.api_key", "media.stt", self.api_key.as_ref())?;
        required("media.stt.model", "media.stt", self.model.as_ref())?;
        if let Some(base_url) = &self.base_url {
            require_https("media.stt.base_url", base_url)?;
        }
        Ok(())
    }
}

impl WebFetch {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.order.is_empty() {
            return Err(invalid("web.fetch.order must not be empty"));
        }
        for name in &self.order {
            if !is_fetch_provider(name) {
                return Err(invalid(format!(
                    "web.fetch.order contains unknown provider `{name}`"
                )));
            }
        }
        for name in self.providers.keys() {
            if !is_fetch_provider(name) {
                return Err(invalid(format!(
                    "web.fetch.providers contains unknown provider `{name}`"
                )));
            }
        }
        Ok(())
    }
}

fn is_fetch_provider(name: &str) -> bool {
    matches!(name, "jina" | "http" | "https" | "tinyfish" | "firecrawl" | "deepseek")
}

impl WebSearch {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.order.is_empty() {
            return Err(invalid("web.search.order must not be empty"));
        }
        for name in &self.order {
            if !is_search_provider(name) {
                return Err(invalid(format!(
                    "web.search.order contains unknown provider `{name}`"
                )));
            }
        }
        if !(1..=3).contains(&self.parallel) {
            return Err(invalid("web.search.parallel must be between 1 and 3"));
        }
        if !(1..=20).contains(&self.results) {
            return Err(invalid("web.search.results must be between 1 and 20"));
        }
        if !(1..=50).contains(&self.max_results) {
            return Err(invalid("web.search.max_results must be between 1 and 50"));
        }
        for name in self.providers.keys() {
            if !is_search_provider(name) {
                return Err(invalid(format!(
                    "web.search.providers contains unknown provider `{name}`"
                )));
            }
        }
        Ok(())
    }
}

fn is_search_provider(name: &str) -> bool {
    matches!(
        name,
        "exa" | "brave" | "deepseek" | "firecrawl" | "tinyfish"
    )
}

impl Tasks {
    fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=8).contains(&self.max_concurrent) {
            return Err(invalid("tasks.max_concurrent must be between 1 and 8"));
        }
        if !(1..=2).contains(&self.max_depth) {
            return Err(invalid("tasks.max_depth must be 1 or 2"));
        }
        positive("tasks.retention_days", u64::from(self.retention_days))?;
        positive("tasks.retention_runs", u64::from(self.retention_runs))?;
        Ok(())
    }
}

impl Models {
    fn validate(&self) -> Result<(), ConfigError> {
        require("models.kind", &self.kind)?;
        if self.kind != "openai-compatible" {
            return Err(invalid("models.kind must be `openai-compatible`"));
        }
        require("models.base_url", &self.base_url)?;
        if !(self.base_url.starts_with("https://") || self.base_url.starts_with("http://")) {
            return Err(invalid("models.base_url must be an http or https URL"));
        }
        require("models.api_key", &self.api_key)?;
        require("models.model", &self.model)?;
        if let Some(header) = &self.session_header {
            require("models.session_header", header)?;
        }
        for (name, value) in &self.headers {
            require("models.headers key", name)?;
            require("models.headers value", value)?;
        }
        Ok(())
    }
}
