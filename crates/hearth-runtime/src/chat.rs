//! A minimal line-based local conversation.

use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;

use hearth_config::ConfigError;
use hearth_core::{ContextBudget, CoreError, Harness};
use hearth_memory::{MemoryService, MemorySource};
use hearth_model::{OpenAiCompatibleProvider, ProviderError};
use serde_json::json;
use thiserror::Error;
use time::OffsetDateTime;

use crate::agent::{Agent, AgentConfig};
use crate::skills::SkillLibrary;
use crate::tools::{
    ToolBuildError, ToolMediaRoots, build_tools, memory_policy, pricing_from_settings,
};

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
    Tools(#[from] ToolBuildError),
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
    let media = ToolMediaRoots {
        inbound: home.join("media/inbound"),
        outbound: home.join("media/outbound"),
    };

    let harness = Harness::open(home)?;
    let session = harness.session().clone();
    let skills = Arc::new(SkillLibrary::new(home));
    let memory = Arc::new(MemoryService::with_policy(
        session.clone(),
        memory_policy(&settings),
    ));
    let transport_key = format!("local:{}", OffsetDateTime::now_utc().unix_timestamp_nanos());
    let conversation = session.create_conversation("Chat", "local", transport_key.as_str())?;

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
        let memory_source = session
            .latest_user_entry_id(&conversation.id)?
            .map(MemorySource::user);
        let built = build_tools(
            &settings,
            &workspace,
            &spill,
            None,
            Some(Arc::clone(&memory)),
            memory_source,
            Some(Arc::clone(&skills)),
            None,
            &media,
        )?;
        let agent = Agent::new(
            session.clone(),
            Arc::clone(&provider),
            pricing_from_settings(&settings),
            AgentConfig {
                soul: soul.clone(),
                model: models.model.clone(),
                provider_name: models.kind.clone(),
                owner: format!("chat:{}", std::process::id()),
                max_turns: settings.agent.max_turns,
                tools: built.core,
                spill_dir: spill.clone(),
            },
        )
        .with_memory(Arc::clone(&memory))
        .with_skills(Arc::clone(&skills))
        .with_groups(built.groups, built.selection)
        .with_budget(ContextBudget::new(
            models.context_window,
            models.max_output_tokens,
            settings.compaction.soft_compact,
            settings.compaction.hard_compact,
            settings.compaction.keep_recent_tokens,
        ));
        match agent.run_submission(&submitted.submission.id) {
            Ok(outcome) => {
                if let Some(answer) = outcome.answer_text {
                    println!("{answer}");
                } else if let Some(notice) = outcome.notice {
                    println!("{notice}");
                }
                if let Err(error) = agent.compact_soft(&conversation.id) {
                    eprintln!("compaction: {error}");
                }
            }
            Err(error) => println!("error: {error}"),
        }
    }
    Ok(())
}
