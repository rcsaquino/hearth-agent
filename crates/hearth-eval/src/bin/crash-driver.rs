//! A separate process used by the S7 crash suite.
//!
//! The driver performs one unit of work and exits. The test harness sets
//! `HEARTH_CRASH_AT` so the storage or effect layer aborts at a named point.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use hearth_core::{Harness, Save};
use hearth_model::{ChatRequest, Pricing, Provider, ProviderError, ProviderStream, StreamEvent};
use hearth_runtime::{Agent, AgentConfig, EffectWorker};
use hearth_tools::ToolRegistry;
use serde_json::json;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("save") => save(&args),
        Some("submit") => submit(&args),
        Some("effects") => effects(&args),
        Some("agent") => agent(&args),
        _ => bail!("usage: crash-driver save|submit|effects|agent <arguments>"),
    }
}

fn save(args: &[String]) -> Result<()> {
    let home = home(args)?;
    let conversation = args.get(3).context("missing conversation id")?;
    let harness = Harness::open(&home)?;
    let commit = harness
        .session()
        .save(Save::new(conversation).entry("assistant", json!({"text": "saved"})))?;
    println!("{}", commit.seq);
    Ok(())
}

fn submit(args: &[String]) -> Result<()> {
    let home = home(args)?;
    let conversation = args.get(3).context("missing conversation id")?;
    let request_id = args.get(4).context("missing request id")?;
    let harness = Harness::open(&home)?;
    let result =
        harness
            .session()
            .submit(conversation, request_id, "input", json!({"text": "hello"}))?;
    println!("created={}", result.created);
    Ok(())
}

fn effects(args: &[String]) -> Result<()> {
    let home = home(args)?;
    let workspace = PathBuf::from(args.get(3).context("missing workspace path")?);
    let harness = Harness::open(&home)?;
    let worker = EffectWorker::new(harness.session().clone(), workspace);
    let recovered = worker.recover_started()?;
    let processed = worker.run_once()?;
    println!("recovered={recovered} processed={processed}");
    Ok(())
}

fn agent(args: &[String]) -> Result<()> {
    let home = home(args)?;
    let conversation = args.get(3).context("missing conversation id")?;
    let request_id = args.get(4).context("missing request id")?;
    let harness = Harness::open(&home)?;
    let session = harness.session().clone();
    let submitted = session.submit(conversation, request_id, "input", json!({"text": "hello"}))?;
    let config = AgentConfig {
        soul: "Test identity.".to_owned(),
        model: "test-model".to_owned(),
        provider_name: "scripted".to_owned(),
        owner: "crash-driver".to_owned(),
        max_turns: 3,
        tools: ToolRegistry::new(vec![]),
        spill_dir: home.join("spill"),
        voice_tags: false,
        guidance: String::new(),
    };
    let agent = Agent::new(
        session,
        Arc::new(ScriptedProvider),
        Pricing::default(),
        config,
    );
    let outcome = agent.run_submission(&submitted.submission.id)?;
    println!("answer={:?}", outcome.answer_text);
    Ok(())
}

struct ScriptedProvider;

impl Provider for ScriptedProvider {
    fn stream(&self, _request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        let events: Vec<Result<StreamEvent, ProviderError>> = vec![
            Ok(StreamEvent::TextDelta("partial answer".to_owned())),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ];
        Ok(Box::new(events.into_iter()))
    }
}

fn home(args: &[String]) -> Result<PathBuf> {
    Ok(PathBuf::from(args.get(2).context("missing home path")?))
}
