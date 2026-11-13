use std::io::IsTerminal;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let home = resolve_home()?;
    if std::env::var_os("HEARTH_DAEMON").is_some() {
        hearth_runtime::init_logging(&home)?;
        hearth_runtime::daemon(&home)?;
        return Ok(());
    }
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if let [flag] = args.as_slice()
        && let Some(flag) = flag.to_str()
    {
        match flag {
            "--version" | "-V" => {
                println!("hearth {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("Run `hearth` with no arguments to open Hearth.");
                return Ok(());
            }
            _ => {}
        }
    }
    if !args.is_empty() {
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
