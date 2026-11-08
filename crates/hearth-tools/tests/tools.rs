use std::fs;

use hearth_tools::{FileTool, Tool};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn reads_a_file_inside_the_workspace() {
    let home = TempDir::new().expect("temp home");
    let workspace = home.path().join("workspace");
    let spill = home.path().join("spill");
    fs::create_dir_all(workspace.join("notes")).expect("create notes");
    fs::write(workspace.join("notes/a.txt"), "hello\nworld\n").expect("write file");

    let tool = FileTool::new(workspace, spill);
    let output = tool
        .call(&json!({"action": "read", "path": "notes/a.txt"}))
        .expect("read file");

    assert!(!output.is_error);
    assert!(output.content.contains("hello"));
    assert!(output.content.contains("world"));
}
