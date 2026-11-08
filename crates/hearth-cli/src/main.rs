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
    Init,
    /// Run the long-lived agent.
    Daemon,
    /// Check settings, database, and lock without printing secrets.
    Doctor,
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
    match cli.command {
        Command::Init => {
            anyhow::bail!("interactive setup is not implemented; use --non-interactive")
        }
        Command::Daemon => anyhow::bail!("daemon is not implemented yet"),
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
