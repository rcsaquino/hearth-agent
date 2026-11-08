//! Read-only prompt auditing commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hearth_config::{ConfigError, Settings};
use hearth_core::{CoreError, Entry, Harness, PromptInput, build_prompt, request_from_document};
use hearth_memory::MemoryService;
use hearth_model::OpenAiCompatibleProvider;
use serde_json::{Value, json};
use thiserror::Error;

use crate::skills::SkillLibrary;
use crate::tools::{build_tools, memory_policy};

/// Options for `hearth prompt`.
#[derive(Debug, Default, Clone)]
pub struct PromptOptions {
    pub entry_id: Option<String>,
    pub last: bool,
    pub json: bool,
    pub system: bool,
    pub preview: Option<String>,
    pub wire: bool,
}

/// A prompt audit failure.
#[derive(Debug, Error)]
pub enum PromptError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("prompt audit failed: {0}")]
    Provider(String),
    #[error("prompt audit failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("prompt audit: {0}")]
    Invalid(String),
}

/// Print the requested prompt view.
pub fn prompt(home: &Path, options: PromptOptions) -> Result<(), PromptError> {
    if options.system && (options.preview.is_some() || options.entry_id.is_some() || options.last) {
        return Err(PromptError::Invalid(
            "--system cannot be combined with --last, an entry id, or --preview".to_owned(),
        ));
    }
    if options.preview.is_some() && (options.entry_id.is_some() || options.last) {
        return Err(PromptError::Invalid(
            "--preview cannot be combined with --last or an entry id".to_owned(),
        ));
    }
    if options.entry_id.is_some() && options.last {
        return Err(PromptError::Invalid(
            "an entry id cannot be combined with --last".to_owned(),
        ));
    }

    let settings = hearth_config::load(home)?;
    settings.validate_for_run()?;
    let harness = Harness::open(home)?;
    let session = harness.session().clone();
    let skills = Arc::new(SkillLibrary::new(home));
    let memory = Arc::new(MemoryService::with_policy(
        session.clone(),
        memory_policy(&settings),
    ));
    let secrets = secret_values(&settings);
    let soul = read_soul(home, &settings);
    let (workspace, spill) = workspace_paths(home, &settings);
    let built = build_tools(
        &settings,
        &workspace,
        &spill,
        None,
        Some(Arc::clone(&memory)),
        None,
        Some(Arc::clone(&skills)),
        None,
    )
    .map_err(|error| PromptError::Invalid(error.to_string()))?;

    if options.system {
        let document = system_document(&settings, &session, &memory, &built.core, &soul, &skills)?;
        let output = if options.json {
            serde_json::to_string_pretty(&document).unwrap_or_else(|_| document.to_string())
        } else {
            system_text(&document)
        };
        println!("{}", redact(&output, &secrets));
        return Ok(());
    }

    if let Some(text) = options.preview {
        let (request, document) = preview_document(
            &settings,
            &session,
            &memory,
            &built.core,
            &soul,
            &skills,
            &text,
        )?;
        if options.wire {
            let body = wire_body(&settings, &request)?;
            let output = serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());
            println!("{}", redact(&output, &secrets));
        } else {
            let output =
                serde_json::to_string_pretty(&document).unwrap_or_else(|_| document.to_string());
            println!("{}", redact(&output, &secrets));
        }
        return Ok(());
    }

    let blob = match &options.entry_id {
        Some(entry_id) => session.prompt_for_entry(entry_id)?.ok_or_else(|| {
            PromptError::Invalid(format!("no prompt was saved for entry {entry_id}"))
        })?,
        None => session.latest_prompt()?.ok_or_else(|| {
            PromptError::Invalid(
                "no prompt has been saved yet; run a turn first or use --preview".to_owned(),
            )
        })?,
    };
    let document: Value = serde_json::from_str(&blob.text)
        .map_err(|error| PromptError::Invalid(format!("saved prompt is not JSON: {error}")))?;
    if options.wire {
        let request = request_from_document(&document).ok_or_else(|| {
            PromptError::Invalid(
                "this prompt predates model metadata; use --preview --wire instead".to_owned(),
            )
        })?;
        let body = wire_body(&settings, &request)?;
        let output = serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());
        println!("{}", redact(&output, &secrets));
        return Ok(());
    }
    if options.json {
        let output =
            serde_json::to_string_pretty(&document).unwrap_or_else(|_| document.to_string());
        println!("{}", redact(&output, &secrets));
    } else {
        println!("{}", redact(&blob.text, &secrets));
    }
    Ok(())
}

fn read_soul(home: &Path, settings: &Settings) -> String {
    let path = match &settings.identity.soul_file {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => home.join(path),
        None => home.join("SOUL.md"),
    };
    std::fs::read_to_string(path).unwrap_or_else(|_| {
        hearth_core::default_soul(&settings.identity.agent_name, &settings.identity.user_name)
    })
}

fn workspace_paths(home: &Path, settings: &Settings) -> (PathBuf, PathBuf) {
    let workspace = match settings.agent.workspace.clone() {
        Some(path) if path.is_absolute() => path,
        Some(path) => home.join(path),
        None => home.join("workspace"),
    };
    (workspace, home.join("spill"))
}

fn model(settings: &Settings) -> &str {
    settings
        .models
        .as_ref()
        .map(|models| models.model.as_str())
        .unwrap_or_default()
}

fn system_document(
    settings: &Settings,
    session: &hearth_core::Session,
    memory: &MemoryService,
    tools: &hearth_tools::ToolRegistry,
    soul: &str,
    skills: &SkillLibrary,
) -> Result<Value, PromptError> {
    let standing = memory.standing().ok();
    let standing_text = standing
        .as_ref()
        .map(|standing| memory.render_standing(&standing.items))
        .unwrap_or_default();
    let standing_version = standing
        .as_ref()
        .map(|standing| standing.version)
        .unwrap_or(0);
    let prompt = build_prompt(
        PromptInput {
            soul: soul.to_owned(),
            conversation_id: "system".to_owned(),
            model: model(settings).to_owned(),
            tools: tools.specs(),
            standing: standing_text,
            standing_version,
            recall: String::new(),
            skills_index: skills.index().unwrap_or_default(),
            runtime_note: "system prefix".to_owned(),
        },
        &[],
    )?;
    let _ = session;
    Ok(prompt.document)
}

fn preview_document(
    settings: &Settings,
    session: &hearth_core::Session,
    memory: &MemoryService,
    tools: &hearth_tools::ToolRegistry,
    soul: &str,
    skills: &SkillLibrary,
    text: &str,
) -> Result<(hearth_model::ChatRequest, Value), PromptError> {
    let conversation = session.latest_conversation()?;
    let conversation_id = conversation
        .as_ref()
        .map(|conversation| conversation.id.clone())
        .unwrap_or_else(|| "preview".to_owned());
    let mut history = match &conversation {
        Some(conversation) => session.history_for_prompt(&conversation.id)?,
        None => Vec::new(),
    };
    history.push(Entry {
        id: "preview".to_owned(),
        conversation_id: conversation_id.clone(),
        seq: i64::MAX,
        kind: "user".to_owned(),
        content: json!({"text": text}),
        tokens_est: hearth_model::estimate_tokens(text),
        source: None,
        prompt_hash: None,
        created_at: session.now_micros(),
    });
    let standing = memory.standing().ok();
    let standing_text = standing
        .as_ref()
        .map(|standing| memory.render_standing(&standing.items))
        .unwrap_or_default();
    let standing_version = standing
        .as_ref()
        .map(|standing| standing.version)
        .unwrap_or(0);
    let recall = memory
        .recall_read(text)
        .map(|items| memory.render_recall(&items))
        .unwrap_or_default();
    let prompt = build_prompt(
        PromptInput {
            soul: soul.to_owned(),
            conversation_id,
            model: model(settings).to_owned(),
            tools: tools.specs(),
            standing: standing_text,
            standing_version,
            recall,
            skills_index: skills.index().unwrap_or_default(),
            runtime_note: "preview".to_owned(),
        },
        &history,
    )?;
    Ok((prompt.request, prompt.document))
}

fn wire_body(
    settings: &Settings,
    request: &hearth_model::ChatRequest,
) -> Result<Value, PromptError> {
    let models = settings
        .models
        .as_ref()
        .ok_or_else(|| PromptError::Invalid("models settings are required".to_owned()))?;
    let provider = OpenAiCompatibleProvider::new(
        models.base_url.clone(),
        models.api_key.clone(),
        models
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        models.session_header.clone(),
    )
    .map_err(|error| PromptError::Provider(error.to_string()))?;
    provider
        .wire_body(request)
        .map_err(|error| PromptError::Provider(error.to_string()))
}

fn system_text(document: &Value) -> String {
    let tools = serde_json::to_string_pretty(&document["tools"])
        .unwrap_or_else(|_| document["tools"].to_string());
    format!(
        "## Identity\n{}\n\n## Operation\n{}\n\n## Tools\n{}\n\n## Standing memory\n{}\n\n## Skills\n{}",
        document["identity"].as_str().unwrap_or_default(),
        document["operation"].as_str().unwrap_or_default(),
        tools,
        document["standing_memory"].as_str().unwrap_or_default(),
        document["skills_index"].as_str().unwrap_or_default()
    )
}

fn secret_values(settings: &Settings) -> Vec<String> {
    let mut values = Vec::new();
    let mut push = |value: &Option<String>| {
        if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
            values.push(value.to_owned());
        }
    };
    if let Some(models) = &settings.models {
        push(&Some(models.api_key.clone()));
    }
    if let Some(telegram) = &settings.telegram {
        push(&telegram.bot_token);
    }
    if let Some(media) = &settings.media {
        if let Some(service) = &media.image {
            push(&service.api_key);
        }
        if let Some(service) = &media.stt {
            push(&service.api_key);
        }
        if let Some(service) = &media.tts {
            push(&service.api_key);
        }
    }
    if let Some(web) = &settings.web {
        for provider in web.search.providers.values() {
            push(&provider.api_key);
        }
        for provider in web.fetch.providers.values() {
            push(&provider.api_key);
        }
    }
    values
}

fn redact(text: &str, secrets: &[String]) -> String {
    let mut output = text.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            output = output.replace(secret, "[redacted]");
        }
    }
    output
}
