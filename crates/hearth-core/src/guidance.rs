//! Rules from the Hearth home and the directories above the workspace.

use std::fs::File;
use std::io::Read;
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
        let (content, file_truncated) = read_guidance(&path);
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        if file_truncated {
            guidance.truncated = true;
        }
        let header = label.map_or_else(String::new, |label| format!("{label}\n\n"));
        let remaining = MAX_GUIDANCE_TOTAL_BYTES.saturating_sub(guidance.text.len() + header.len());
        if content.len() > remaining {
            let partial = truncate_to(content, remaining);
            guidance.text.push_str(&header);
            guidance.text.push_str(partial);
            guidance.text.push_str("\n\n[guidance truncated at 32 KB]");
            guidance.truncated = true;
            guidance.files.push(path);
            break;
        }
        guidance.text.push_str(&header);
        guidance.text.push_str(content);
        guidance.text.push_str("\n\n");
        guidance.files.push(path);
    }
    guidance.text = guidance.text.trim_end().to_owned();
    guidance
}

fn read_guidance(path: &Path) -> (String, bool) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return (String::new(), false),
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_GUIDANCE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return (String::new(), false);
    }
    let truncated = bytes.len() > MAX_GUIDANCE_FILE_BYTES as usize;
    if truncated {
        bytes.truncate(MAX_GUIDANCE_FILE_BYTES as usize);
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str("\n\n[truncated at 16 KB]");
    }
    (text, truncated)
}

fn truncate_to(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
