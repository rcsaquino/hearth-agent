use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn hearth() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hearth"))
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create skill dirs");
    }
    fs::write(path, text).expect("write skill");
}

fn home() -> TempDir {
    let home = TempDir::new().expect("temp home");
    let body = "## When to use\n\nWhen research is needed.\n\n## Steps\n\nSearch carefully.\n\n## Pitfalls\n\nDo not trust one source.\n";
    write(
        &home.path().join("skills/learned/research.md"),
        &format!(
            "---\nname: research\ndescription: Research a topic\nversion: 2\nstatus: active\ncreated: 2026-01-01\nupdated: 2026-02-01\nlast_used: 2026-02-01\nuses: 3\nsuccesses: 2\nfailures: 1\nevidence: [entry-2]\n---\n\n{body}"
        ),
    );
    write(
        &home.path().join("skills/learned/.history/research/v1.md"),
        &format!(
            "---\nname: research\ndescription: Research a topic\nversion: 1\nstatus: active\ncreated: 2026-01-01\nupdated: 2026-01-01\nlast_used: \nuses: 0\nsuccesses: 0\nfailures: 0\nevidence: []\n---\n\n{body}"
        ),
    );
    home
}

fn run(home: &Path, args: &[&str]) -> String {
    let output = hearth()
        .args(args)
        .arg("--home")
        .arg(home)
        .output()
        .expect("run hearth");
    assert!(
        output.status.success(),
        "hearth {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn skills_commands_read_the_library() {
    let home = home();
    let list = run(home.path(), &["skills", "list"]);
    assert!(list.contains("research"), "{list}");
    assert!(list.contains("active"), "{list}");
    assert!(list.contains("learned"), "{list}");

    let show = run(home.path(), &["skills", "show", "research"]);
    assert!(show.contains("Search carefully."), "{show}");

    let history = run(home.path(), &["skills", "history", "research"]);
    assert!(history.contains("v1"), "{history}");
    assert!(history.contains("v2"), "{history}");

    let diff = run(home.path(), &["skills", "diff", "research"]);
    assert!(diff.contains("v1"), "{diff}");
    assert!(diff.contains("v2"), "{diff}");

    let stats = run(home.path(), &["skills", "stats"]);
    assert!(stats.contains("total 1"), "{stats}");
    assert!(stats.contains("research"), "{stats}");
}

#[test]
fn skills_list_reports_an_empty_library() {
    let home = TempDir::new().expect("temp home");
    let list = run(home.path(), &["skills", "list"]);
    assert!(list.contains("no skills"), "{list}");
}
