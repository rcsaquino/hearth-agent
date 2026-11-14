//! Rebuild the home layout from `settings.json`.
//!
//! Onboarding writes the folders, `SOUL.md`, and an empty `HEARTH.md` once. If
//! the home loses everything but its settings, the daemon re-creates the same
//! state on the next start, so the record needs no second wizard. The database
//! stays the daemon's own write; this module never opens it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

/// The folders onboarding writes and the daemon expects to exist.
const HOME_DIRECTORIES: [&str; 8] = [
    "workspace",
    "skills",
    "skills/learned",
    "media/inbound",
    "media/outbound",
    "spill",
    "logs",
    "backups",
];

/// The files a restore created. Folders follow from the fixed layout.
#[derive(Debug, Default, Clone)]
pub struct HomeRestore {
    pub created: Vec<PathBuf>,
}

impl HomeRestore {
    /// True when every file was already on disk.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum HomeError {
    #[error(transparent)]
    Config(#[from] hearth_config::ConfigError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Recreate every folder and file onboarding writes, keeping what already exists.
pub fn restore_home(home: &Path) -> Result<HomeRestore, HomeError> {
    let settings = hearth_config::load(home)?;
    Ok(scaffold(home, &settings.identity.agent_name)?)
}

/// Write the layout for a known agent name. `init` and `restore_home` share it.
pub(crate) fn scaffold(home: &Path, agent_name: &str) -> io::Result<HomeRestore> {
    fs::create_dir_all(home)?;
    for directory in HOME_DIRECTORIES {
        fs::create_dir_all(home.join(directory))?;
    }
    let mut created = Vec::new();
    let soul = hearth_core::default_soul(agent_name);
    if create_private(&home.join("SOUL.md"), soul.as_bytes())? {
        created.push(home.join("SOUL.md"));
    }
    if create_private(&home.join(hearth_core::HEARTH_GUIDANCE_FILE), b"")? {
        created.push(home.join(hearth_core::HEARTH_GUIDANCE_FILE));
    }
    Ok(HomeRestore { created })
}

fn create_private(path: &Path, contents: &[u8]) -> io::Result<bool> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    };
    file.write_all(contents)?;
    file.flush()?;
    Ok(true)
}
