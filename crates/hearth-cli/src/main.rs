use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "hearth", version, about = "A durable personal agent")]
struct Cli {
    /// Home folder. Defaults to HEARTH_HOME, then ~/.hearth.
    #[arg(long, global = true, value_name = "PATH")]
    home: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a home folder and a working settings file.
    Init(InitArgs),
    /// Run the long-lived agent.
    Daemon,
    /// Talk to the agent from this terminal.
    Chat,
    /// Follow live events from the running daemon.
    Watch(WatchArgs),
    /// Ask the running daemon for its status.
    Status(StatusArgs),
    /// Show usage and cost summaries from the local ledger.
    Stats(StatsArgs),
    /// Inspect the exact prompt sent to the provider.
    Prompt(PromptArgs),
    /// Check settings, database, and lock without printing secrets.
    Doctor,
    /// Run the offline evaluation suite.
    Eval(EvalArgs),
}

#[derive(Debug, clap::Args)]
struct EvalArgs {
    #[command(subcommand)]
    command: EvalCommand,
}

#[derive(Debug, clap::Subcommand)]
enum EvalCommand {
    /// Run the memory recall benchmark.
    Memory(MemoryEvalArgs),
    /// Run the 100-turn token-efficiency benchmark.
    Tokens(TokensEvalArgs),
}

#[derive(Debug, clap::Args)]
struct MemoryEvalArgs {
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// Also run the large-library retrieval check, for example 200k.
    #[arg(long, value_name = "COUNT")]
    scale: Option<String>,
}

#[derive(Debug, clap::Args)]
struct TokensEvalArgs {
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct WatchArgs {
    /// Only show events for one task.
    task_id: Option<String>,
    /// Print one JSON object per line.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct PromptArgs {
    /// Show the prompt saved for this entry id.
    entry_id: Option<String>,
    /// Show the newest saved prompt.
    #[arg(long)]
    last: bool,
    /// Print the audit document as JSON.
    #[arg(long)]
    json: bool,
    /// Print the stable system prefix only.
    #[arg(long)]
    system: bool,
    /// Assemble a hypothetical next prompt without saving or calling the provider.
    #[arg(long, value_name = "TEXT")]
    preview: Option<String>,
    /// Print the OpenAI-compatible wire body without sending it.
    #[arg(long)]
    wire: bool,
}

#[derive(Debug, clap::Args)]
struct StatusArgs {
    /// Print the raw status object.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct StatsArgs {
    /// Print the summary as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct InitArgs {
    /// Do not prompt; every required value comes from a flag.
    #[arg(long)]
    non_interactive: bool,
    /// Telegram bot token.
    #[arg(long)]
    telegram_token: Option<String>,
    /// Allowed Telegram user id. Repeat for more than one user.
    #[arg(long = "allowed-user-id", value_name = "ID")]
    allowed_user_ids: Vec<i64>,
    /// OpenAI-compatible provider base URL.
    #[arg(long)]
    base_url: Option<String>,
    /// Provider API key, or a ${ENV_VAR} reference.
    #[arg(long)]
    api_key: Option<String>,
    /// Model name.
    #[arg(long)]
    model: Option<String>,
    /// Agent name.
    #[arg(long)]
    agent_name: Option<String>,
    /// User name.
    #[arg(long)]
    user_name: Option<String>,
    /// IANA timezone name.
    #[arg(long)]
    timezone: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(cli) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let home = resolve_home(cli.home)?;
    if matches!(&cli.command, Command::Daemon) {
        hearth_runtime::init_logging(&home)?;
    }
    match cli.command {
        Command::Init(args) => hearth_runtime::init(
            &home,
            hearth_runtime::InitOptions {
                non_interactive: args.non_interactive,
                telegram_token: args.telegram_token,
                allowed_user_ids: args.allowed_user_ids,
                base_url: args.base_url,
                api_key: args.api_key,
                model: args.model,
                agent_name: args.agent_name,
                user_name: args.user_name,
                timezone: args.timezone,
            },
        )?,
        Command::Daemon => hearth_runtime::daemon(&home)?,
        Command::Chat => hearth_runtime::chat(&home)?,
        Command::Watch(args) => hearth_runtime::watch(&home, args.task_id, args.json)?,
        Command::Status(args) => hearth_runtime::status(&home, args.json)?,
        Command::Stats(args) => hearth_runtime::stats(&home, args.json)?,
        Command::Prompt(args) => hearth_runtime::prompt(
            &home,
            hearth_runtime::PromptOptions {
                entry_id: args.entry_id,
                last: args.last,
                json: args.json,
                system: args.system,
                preview: args.preview,
                wire: args.wire,
            },
        )?,
        Command::Doctor => hearth_runtime::doctor(&home)?,
        Command::Eval(args) => run_eval(args)?,
    }
    Ok(())
}

fn run_eval(args: EvalArgs) -> anyhow::Result<()> {
    match args.command {
        EvalCommand::Memory(args) => {
            let mut report = hearth_eval::memory_bench::run()?;
            if let Some(scale) = args.scale.as_deref() {
                report.scale = Some(hearth_eval::memory_bench::run_scale(parse_scale(scale)?)?);
            }
            if args.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_memory_report(&report);
            }
            if report.secret_leaks > 0 || report.duplicate_failures > 0 {
                anyhow::bail!("memory benchmark invariants failed");
            }
            Ok(())
        }
        EvalCommand::Tokens(args) => {
            let report = hearth_eval::tokens_bench::run()?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_tokens_report(&report);
            }
            Ok(())
        }
    }
}

fn print_tokens_report(report: &hearth_eval::tokens_bench::TokensReport) {
    println!(
        "token benchmark: {} turns, {} compactions, {} summary tokens",
        report.turns, report.compactions, report.summary_tokens
    );
    for target in &report.targets {
        println!(
            "{}: {:.3} {} (target {:.3}) {}",
            target.name,
            target.measured,
            target.unit,
            target.target,
            if target.met { "met" } else { "missed" }
        );
    }
    println!(
        "fixed overhead p95/max: {}/{} tokens; prompt tokens p95: {}; cache hit turns: {}",
        report.fixed_overhead_p95,
        report.fixed_overhead_max,
        report.prompt_tokens_p95,
        report.cache_hit_turns
    );
    for (name, total) in &report.token_totals {
        println!("  {name}: {total} tokens");
    }
    for note in &report.deferred {
        println!("deferred: {note}");
    }
}

fn parse_scale(value: &str) -> anyhow::Result<usize> {
    let text = value.trim().to_ascii_lowercase();
    let number = text.strip_suffix('k').unwrap_or(&text);
    let count: usize = number
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid scale count: {value}"))?;
    let count = if text.ends_with('k') {
        count
            .checked_mul(1_000)
            .ok_or_else(|| anyhow::anyhow!("scale count is too large: {value}"))?
    } else {
        count
    };
    if count == 0 {
        anyhow::bail!("scale count must be positive");
    }
    Ok(count)
}

fn print_memory_report(report: &hearth_eval::memory_bench::BenchReport) {
    println!(
        "memory benchmark: {} questions, {} memories, {} stored rows",
        report.questions, report.fixture_memories, report.stored_rows
    );
    println!(
        "recall@10: {:.3} (target {:.3})",
        report.recall_at_10, report.targets.recall_at_10
    );
    println!(
        "precision@5: {:.3} (target {:.3})",
        report.precision_at_5, report.targets.precision_at_5
    );
    println!(
        "recall tokens p95: {} (target {})",
        report.recall_tokens_p95, report.targets.recall_tokens_p95
    );
    println!(
        "retrieval p95: {:.1} ms (target {:.1} ms)",
        report.retrieval_ms_p95, report.targets.retrieval_ms_p95
    );
    for (name, category) in &report.categories {
        println!(
            "  {name}: {} questions, recall@10 {:.3}, precision@5 {:.3}",
            category.count, category.recall_at_10, category.precision_at_5
        );
    }
    println!(
        "temporal: {}/{} correct",
        report.temporal.correct, report.temporal.count
    );
    println!(
        "supersession: {}/{} correct",
        report.supersession.correct, report.supersession.count
    );
    println!("duplicate failures: {}", report.duplicate_failures);
    println!("secret leaks: {}", report.secret_leaks);
    if let Some(scale) = &report.scale {
        println!(
            "scale: {} items ({} fixture + {} noise), {} queries, {} matched, recall_read p95 {:.1} ms, real recall p95 {:.1} ms (target {:.1} ms)",
            scale.items,
            scale.fixture_rows,
            scale.noise_rows,
            scale.queries,
            scale.matched,
            scale.retrieval_ms_p95,
            scale.real_recall_ms_p95,
            scale.target_ms
        );
    }
}

fn resolve_home(argument: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(home) = argument {
        return Ok(home);
    }
    if let Ok(home) = std::env::var("HEARTH_HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home));
    }
    let home =
        std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set; pass --home"))?;
    Ok(PathBuf::from(home).join(".hearth"))
}
