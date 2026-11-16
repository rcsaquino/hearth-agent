use std::fs;

use hearth_core::load_guidance;
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
}

#[test]
fn every_guidance_file_is_read_whole() {
    let (_root, home, workspace) = home_and_workspace();
    let large = format!("{}\nTHE LAST INSTRUCTION", "A".repeat(20 * 1024));
    fs::write(workspace.join("AGENTS.md"), &large).expect("large rules");
    let guidance = load_guidance(&home, &workspace);
    assert!(
        guidance.text.contains("THE LAST INSTRUCTION"),
        "the last line of a large file must reach the prompt"
    );
    assert!(
        !guidance.text.contains("truncated"),
        "a guidance file is never cut: {}",
        guidance.text.len()
    );
    assert_eq!(guidance.files.len(), 1);
}

#[test]
fn the_total_guidance_is_not_capped() {
    let (root, home, workspace) = home_and_workspace();
    let project = format!("{}\nPROJECT TAIL", "B".repeat(20 * 1024));
    let nested = format!("{}\nNESTED TAIL", "C".repeat(20 * 1024));
    fs::write(root.path().join("project/AGENTS.md"), &project).expect("project rules");
    fs::write(workspace.join("AGENTS.md"), &nested).expect("nested rules");
    let guidance = load_guidance(&home, &workspace);
    assert!(
        guidance.text.contains("PROJECT TAIL"),
        "the wider project rules reach the prompt"
    );
    assert!(
        guidance.text.contains("NESTED TAIL"),
        "the most specific rules reach the prompt last"
    );
    assert!(!guidance.text.contains("truncated"));
    assert_eq!(guidance.files.len(), 2);
}

#[test]
fn a_directory_named_agents_is_skipped() {
    let (_root, home, workspace) = home_and_workspace();
    fs::create_dir(workspace.join("AGENTS.md")).expect("directory");
    let guidance = load_guidance(&home, &workspace);
    assert!(guidance.text.is_empty());
}
