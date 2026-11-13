use std::fs;

use hearth_core::{MAX_GUIDANCE_FILE_BYTES, MAX_GUIDANCE_TOTAL_BYTES, load_guidance};
use tempfile::TempDir;

fn home_and_workspace() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = TempDir::new().expect("temp root");
    let home = root.path().join("home");
    let workspace = root.path().join("project/nested");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&workspace).expect("workspace");
    (root, home, workspace)
}

#[test]
fn home_and_project_guidance_are_loaded_in_order() {
    let (root, home, workspace) = home_and_workspace();
    fs::write(home.join("HEARTH.md"), "HOME RULE").expect("home rules");
    fs::write(root.path().join("project/AGENTS.md"), "PROJECT RULE").expect("project rules");
    fs::write(workspace.join("AGENTS.md"), "NESTED RULE").expect("nested rules");

    let guidance = load_guidance(&home, &workspace);
    assert!(!guidance.truncated);
    let home_at = guidance.text.find("HOME RULE").expect("home text");
    let project_at = guidance.text.find("PROJECT RULE").expect("project text");
    let nested_at = guidance.text.find("NESTED RULE").expect("nested text");
    assert!(home_at < project_at, "home rules must come first");
    assert!(project_at < nested_at, "the nearest project must come last");
    assert!(
        !guidance.text.contains("Hearth home"),
        "the home file header leaked into the prompt: {}",
        guidance.text
    );
    assert!(
        !guidance.text.contains(&home.display().to_string()),
        "an absolute home path leaked into the prompt: {}",
        guidance.text
    );
    assert_eq!(
        guidance.text.matches("### Project rules").count(),
        2,
        "each project file should get one compact label: {}",
        guidance.text
    );
    assert!(
        guidance
            .files
            .iter()
            .any(|path| path.ends_with("HEARTH.md"))
    );
    assert!(
        guidance
            .files
            .iter()
            .any(|path| path.ends_with("project/AGENTS.md"))
    );
}

#[test]
fn missing_guidance_is_empty() {
    let (_root, home, workspace) = home_and_workspace();
    let guidance = load_guidance(&home, &workspace);
    assert!(guidance.text.is_empty());
    assert!(guidance.files.is_empty());
    assert!(!guidance.truncated);
}

#[test]
fn a_large_file_is_truncated() {
    let (_root, home, workspace) = home_and_workspace();
    let large = "A".repeat(MAX_GUIDANCE_FILE_BYTES as usize + 4096);
    fs::write(workspace.join("AGENTS.md"), &large).expect("large rules");
    let guidance = load_guidance(&home, &workspace);
    assert!(guidance.truncated);
    assert!(guidance.text.contains("truncated"));
    assert!(
        guidance.text.len() < large.len(),
        "large file was not truncated"
    );
}

#[test]
fn the_total_guidance_is_capped() {
    let (root, home, workspace) = home_and_workspace();
    let large = "B".repeat(MAX_GUIDANCE_FILE_BYTES as usize + 4096);
    fs::write(root.path().join("project/AGENTS.md"), &large).expect("project rules");
    fs::write(workspace.join("AGENTS.md"), &large).expect("nested rules");
    let guidance = load_guidance(&home, &workspace);
    assert!(guidance.truncated);
    assert!(
        guidance.text.len() <= MAX_GUIDANCE_TOTAL_BYTES + 200,
        "total guidance was {} bytes",
        guidance.text.len()
    );
}

#[test]
fn a_directory_named_agents_is_skipped() {
    let (_root, home, workspace) = home_and_workspace();
    fs::create_dir(workspace.join("AGENTS.md")).expect("directory");
    let guidance = load_guidance(&home, &workspace);
    assert!(guidance.text.is_empty());
}
