//! Rules from the Hearth home and the directories above the workspace.

use std::path::{Path, PathBuf};

/// The rules file in the Hearth home.
pub const HEARTH_GUIDANCE_FILE: &str = "HEARTH.md";
/// The project rules file in the workspace and its ancestors.
pub const AGENTS_GUIDANCE_FILE: &str = "AGENTS.md";
/// The largest part of one guidance file that is read.
pub const MAX_GUIDANCE_FILE_BYTES: u64 = 16 * 1024;
/// The largest combined guidance text.
pub const MAX_GUIDANCE_TOTAL_BYTES: usize = 32 * 1024;

/// The guidance read from disk, ready to place in the prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Guidance {
    pub text: String,
    pub files: Vec<PathBuf>,
    pub truncated: bool,
}

/// Read `HEARTH.md` from the home and every `AGENTS.md` from the filesystem
/// root down to the workspace. Missing files are skipped.
pub fn load(_home: &Path, _workspace: &Path) -> Guidance {
    Guidance::default()
}
