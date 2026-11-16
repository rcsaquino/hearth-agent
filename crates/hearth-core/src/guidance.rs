//! Rules from the Hearth home and the directories above the workspace.

use std::fs;
use std::path::{Path, PathBuf};

/// The rules file in the Hearth home.
pub const HEARTH_GUIDANCE_FILE: &str = "HEARTH.md";
/// The project rules file in the workspace and its ancestors.
pub const AGENTS_GUIDANCE_FILE: &str = "AGENTS.md";

/// One guidance file and the bytes read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceFile {
    pub path: PathBuf,
    pub bytes: u64,
}

/// The guidance read from disk, ready to place in the prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Guidance {
    pub text: String,
    pub files: Vec<GuidanceFile>,
}

/// Read `HEARTH.md` from the home and every `AGENTS.md` from the filesystem
/// root down to the workspace. Missing files are skipped and no file is cut.
pub fn load(home: &Path, workspace: &Path) -> Guidance {
    let mut sources: Vec<(Option<&str>, PathBuf)> = Vec::new();
    let hearth = home.join(HEARTH_GUIDANCE_FILE);
    if hearth.is_file() {
        // The home rules are the base layer; a path header would only add noise.
        sources.push((None, hearth));
    }

    let workspace = if workspace.is_absolute() {
        workspace.to_path_buf()
    } else {
        std::path::absolute(workspace).unwrap_or_else(|_| workspace.to_path_buf())
    };
    let mut directories: Vec<PathBuf> = workspace.ancestors().map(Path::to_path_buf).collect();
    directories.reverse();
    for directory in directories {
        let agents = directory.join(AGENTS_GUIDANCE_FILE);
        if agents.is_file() {
            sources.push((Some("### Project rules"), agents));
        }
    }

    let mut guidance = Guidance::default();
    for (label, path) in sources {
        let (content, bytes) = read_guidance(&path);
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        let header = label.map_or_else(String::new, |label| format!("{label}\n\n"));
        guidance.text.push_str(&header);
        guidance.text.push_str(content);
        guidance.text.push_str("\n\n");
        guidance.files.push(GuidanceFile { path, bytes });
    }
    guidance.text = guidance.text.trim_end().to_owned();
    guidance
}

fn read_guidance(path: &Path) -> (String, u64) {
    let Ok(bytes) = fs::read(path) else {
        return (String::new(), 0);
    };
    let size = bytes.len() as u64;
    (String::from_utf8_lossy(&bytes).into_owned(), size)
}
