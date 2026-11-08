//! A separate process used by the S7 crash suite.
//!
//! The driver performs one unit of work and exits. The test harness sets
//! `HEARTH_CRASH_AT` so the storage or effect layer aborts at a named point.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hearth_core::{Harness, Save};
use hearth_runtime::EffectWorker;
use serde_json::json;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("save") => save(&args),
        Some("submit") => submit(&args),
        Some("effects") => effects(&args),
        _ => bail!("usage: crash-driver save|submit|effects <arguments>"),
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
    let result = harness.session().submit(
        conversation,
        request_id,
        "input",
        json!({"text": "hello"}),
    )?;
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

fn home(args: &[String]) -> Result<PathBuf> {
    Ok(PathBuf::from(args.get(2).context("missing home path")?))
}
