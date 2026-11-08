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
    /// Check settings, database, and lock without printing secrets.
    Doctor,
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
        Command::Doctor => hearth_runtime::doctor(&home)?,
    }
    Ok(())
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
