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

#[test]
fn writes_edits_and_lists_inside_the_workspace() {
    let home = TempDir::new().expect("temp home");
    let workspace = home.path().join("workspace");
    let spill = home.path().join("spill");
    let tool = FileTool::new(&workspace, &spill);

    tool.call(&json!({"action": "write", "path": "notes/b.txt", "content": "one\n"}))
        .expect("write");
    let read = tool
        .call(&json!({"action": "read", "path": "notes/b.txt"}))
        .expect("read");
    assert_eq!(read.content, "one\n");

    tool.call(&json!({
        "action": "edit",
        "path": "notes/b.txt",
        "old": "one",
        "new": "two"
    }))
    .expect("edit");
    let read = tool
        .call(&json!({"action": "read", "path": "notes/b.txt"}))
        .expect("read after edit");
    assert_eq!(read.content, "two\n");

    let list = tool
        .call(&json!({"action": "list", "path": "notes"}))
        .expect("list");
    assert!(list.content.contains("file\tb.txt\t4"), "{}", list.content);
}

#[test]
fn rejects_path_escapes() {
    let home = TempDir::new().expect("temp home");
    let workspace = home.path().join("workspace");
    let spill = home.path().join("spill");
    fs::create_dir_all(&workspace).expect("create workspace");
    let tool = FileTool::new(&workspace, &spill);

    let error = tool
        .call(&json!({"action": "write", "path": "../escape.txt", "content": "bad"}))
        .expect_err("parent escape must fail");
    assert_eq!(error.code, "invalid_path");

    let error = tool
        .call(&json!({"action": "read", "path": "/etc/passwd"}))
        .expect_err("absolute path must fail");
    assert_eq!(error.code, "invalid_path");

    fs::write(home.path().join("outside.txt"), "outside").expect("write outside");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(home.path().join("outside.txt"), workspace.join("link.txt"))
            .expect("symlink");
        let error = tool
            .call(&json!({"action": "write", "path": "link.txt", "content": "bad"}))
            .expect_err("symlink escape must fail");
        assert_eq!(error.code, "path_escape");
        assert_eq!(
            fs::read_to_string(home.path().join("outside.txt")).expect("read outside"),
            "outside"
        );
    }
}

#[test]
fn reads_spill_paths() {
    let home = TempDir::new().expect("temp home");
    let workspace = home.path().join("workspace");
    let spill = home.path().join("spill");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&spill).expect("create spill");
    fs::write(spill.join("out.txt"), "full output").expect("write spill");

    let tool = FileTool::new(&workspace, &spill);
    let output = tool
        .call(&json!({"action": "read", "path": "spill/out.txt"}))
        .expect("read spill");
    assert_eq!(output.content, "full output");
}
