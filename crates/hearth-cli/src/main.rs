use std::io::IsTerminal;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let home = resolve_home()?;
    if std::env::var_os("HEARTH_DAEMON").is_some() {
        hearth_runtime::init_logging(&home)?;
        hearth_runtime::daemon(&home)?;
        return Ok(());
    }
    if std::env::args_os().len() > 1 {
        eprintln!("Run `hearth` with no arguments to open Hearth.");
        std::process::exit(2);
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        anyhow::bail!("Hearth needs an interactive terminal; run `hearth` from a terminal.");
    }
    hearth_cli::app::run(home)
}

fn resolve_home() -> anyhow::Result<PathBuf> {
    if let Ok(home) = std::env::var("HEARTH_HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home));
    }
    let home =
        std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set; set HEARTH_HOME"))?;
    Ok(PathBuf::from(home).join(".hearth"))
}
