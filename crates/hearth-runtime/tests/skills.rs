use std::fs;
use std::path::Path;
use std::sync::Arc;

use hearth_runtime::skills::{SkillLibrary, SkillStatus, SkillTool};
use hearth_tools::Tool;
use serde_json::json;
use tempfile::TempDir;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create skill directory");
    }
    fs::write(path, text).expect("write skill");
}

fn user_skill(home: &Path, name: &str, description: &str, body: &str) {
    write(
        &home.join("skills").join(format!("{name}.md")),
        &format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n"),
    );
}

fn learned_skill(
    home: &Path,
    name: &str,
    status: &str,
    uses: u64,
    successes: u64,
    last_used: &str,
    description: &str,
) {
    write(
        &home.join("skills/learned").join(format!("{name}.md")),
        &format!(
            "---\nname: {name}\ndescription: {description}\nversion: 1\nstatus: {status}\ncreated: 2026-01-01\nupdated: 2026-01-01\nlast_used: {last_used}\nuses: {uses}\nsuccesses: {successes}\nfailures: 0\nevidence: [entry-1]\n---\n\n## When to use\n\nWhen it helps.\n\n## Steps\n\nDo the thing.\n\n## Pitfalls\n\nDo not skip.\n"
        ),
    );
}

fn body() -> &'static str {
    "## When to use\n\nWhen a new procedure is needed.\n\n## Steps\n\nDo it carefully.\n\n## Pitfalls\n\nDo not rush."
}

#[test]
fn a_user_skill_and_a_learned_skill_are_indexed() {
    let home = TempDir::new().expect("temp home");
    user_skill(
        home.path(),
        "writing",
        "Write clearly",
        "## When to use\n\nAlways.\n\n## Steps\n\nWrite.\n\n## Pitfalls\n\nDo not ramble.",
    );
    learned_skill(
        home.path(),
        "research",
        "active",
        3,
        2,
        "2026-02-01",
        "Research a topic",
    );

    let library = SkillLibrary::new(home.path());
    let index = library.index().expect("index");
    assert!(index.contains("writing: Write clearly"), "{index}");
    assert!(index.contains("research: Research a topic"), "{index}");

    let skills = library.load_all().expect("load");
    assert_eq!(skills.len(), 2);
    assert!(
        skills.iter().any(|skill| skill.name == "writing"
            && skill.scope == hearth_runtime::skills::SkillScope::User)
    );
    assert!(skills.iter().any(|skill| skill.name == "research"
        && skill.scope == hearth_runtime::skills::SkillScope::Learned));
}

#[test]
fn a_skill_without_frontmatter_uses_the_file_name_and_first_body_line() {
    let home = TempDir::new().expect("temp home");
    write(
        &home.path().join("skills/simple.md"),
        "# A simple skill\n\nUse this when simple work appears.\n\n## Steps\n\nDo it.\n",
    );
    let library = SkillLibrary::new(home.path());
    let skills = library.load_all().expect("load");
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "simple");
    assert!(
        skills[0].description.contains("simple work"),
        "{:?}",
        skills[0].description
    );
}

#[test]
fn malformed_frontmatter_is_rejected() {
    let home = TempDir::new().expect("temp home");
    write(
        &home.path().join("skills/learned/broken.md"),
        "---\nname: broken\ndescription: no closing\n## When to use\n",
    );
    let library = SkillLibrary::new(home.path());
    let error = library.load_all().expect_err("malformed must fail");
    assert!(error.to_string().contains("frontmatter"), "{error}");
}

#[test]
fn the_index_archives_low_success_learned_skills_over_the_cap() {
    let home = TempDir::new().expect("temp home");
    for index in 0..60 {
        let name = format!("skill-{index:02}");
        let successes = if index < 5 { 0 } else { 3 };
        learned_skill(
            home.path(),
            &name,
            "active",
            4,
            successes,
            "2026-01-01",
            "d",
        );
    }
    let library = SkillLibrary::new(home.path());
    let read_only = library.read_index().expect("read index");
    assert!(
        library
            .load_all()
            .expect("load")
            .iter()
            .all(|skill| skill.status != SkillStatus::Archived),
        "read_index must not archive"
    );
    assert!(
        read_only
            .lines()
            .filter(|line| line.starts_with("- "))
            .count()
            <= 50,
        "read_index exceeded the count cap"
    );
    let index = library.index().expect("index");
    let lines = index.lines().filter(|line| line.starts_with("- ")).count();
    assert!(lines <= 50, "index has {lines} lines");
    assert!(
        hearth_model::estimate_tokens(&index) <= 300,
        "index is {} tokens",
        hearth_model::estimate_tokens(&index)
    );
    let archived = library
        .load_all()
        .expect("load")
        .into_iter()
        .filter(|skill| skill.status == SkillStatus::Archived)
        .count();
    assert!(archived > 0, "no learned skill was archived");
    assert!(
        archived >= 5,
        "the zero-success skills must be archived first"
    );
}

#[test]
fn the_tool_loads_a_skill_and_records_use() {
    let home = TempDir::new().expect("temp home");
    learned_skill(
        home.path(),
        "research",
        "active",
        0,
        0,
        "",
        "Research a topic",
    );
    let library = Arc::new(SkillLibrary::new(home.path()));
    let tool = SkillTool::new(Arc::clone(&library));

    let output = tool
        .call(&json!({"action": "load", "name": "research"}))
        .expect("load skill");
    assert!(
        output.content.contains("Do the thing."),
        "{}",
        output.content
    );
    let skill = library.find("research").expect("find").expect("exists");
    assert_eq!(skill.uses, 1);
    assert!(skill.last_used.is_some());
}

#[test]
fn the_tool_saves_versions_with_history_and_changelog() {
    let home = TempDir::new().expect("temp home");
    let library = Arc::new(SkillLibrary::new(home.path()));
    let tool = SkillTool::new(Arc::clone(&library));

    let first = tool
        .call(&json!({
            "action": "save",
            "name": "new_skill",
            "description": "A new procedure",
            "body": body()
        }))
        .expect("save");
    assert!(first.content.contains("v1"), "{}", first.content);
    let skill = library.find("new_skill").expect("find").expect("exists");
    assert_eq!(skill.version, 1);
    assert_eq!(skill.status, SkillStatus::Draft);

    let second = tool
        .call(&json!({
            "action": "save",
            "name": "new_skill",
            "description": "A revised procedure",
            "body": body()
        }))
        .expect("save revision");
    assert!(second.content.contains("v2"), "{}", second.content);
    let history = library.history("new_skill").expect("history");
    assert_eq!(history, vec![1, 2]);
    let old = library
        .version_body("new_skill", 1)
        .expect("old version")
        .expect("old body");
    assert!(old.contains("A new procedure"), "{old}");
    let changelog =
        fs::read_to_string(home.path().join("skills/learned/CHANGELOG.md")).expect("changelog");
    assert!(changelog.contains("new_skill"), "{changelog}");
}

#[test]
fn the_tool_rejects_writes_to_a_user_skill() {
    let home = TempDir::new().expect("temp home");
    user_skill(home.path(), "writing", "Write clearly", body());
    let library = Arc::new(SkillLibrary::new(home.path()));
    let tool = SkillTool::new(Arc::clone(&library));

    let error = tool
        .call(&json!({
            "action": "save",
            "name": "writing",
            "description": "Overwrite",
            "body": body()
        }))
        .expect_err("user skill must be read-only");
    assert!(
        error.to_string().contains("read-only") || error.to_string().contains("learned"),
        "{error}"
    );

    let error = tool
        .call(&json!({"action": "archive", "name": "writing"}))
        .expect_err("user skill cannot be archived");
    assert!(
        error.to_string().contains("read-only") || error.to_string().contains("learned"),
        "{error}"
    );
}

#[test]
fn the_tool_archives_a_learned_skill() {
    let home = TempDir::new().expect("temp home");
    learned_skill(
        home.path(),
        "research",
        "active",
        0,
        0,
        "",
        "Research a topic",
    );
    let library = Arc::new(SkillLibrary::new(home.path()));
    let tool = SkillTool::new(Arc::clone(&library));

    tool.call(&json!({"action": "archive", "name": "research"}))
        .expect("archive");
    let skill = library.find("research").expect("find").expect("exists");
    assert_eq!(skill.status, SkillStatus::Archived);
    assert!(!library.index().expect("index").contains("research"));
}

#[test]
fn saving_without_the_three_sections_is_rejected() {
    let home = TempDir::new().expect("temp home");
    let library = Arc::new(SkillLibrary::new(home.path()));
    let tool = SkillTool::new(library);
    let error = tool
        .call(&json!({
            "action": "save",
            "name": "bad",
            "description": "Bad",
            "body": "Only one section."
        }))
        .expect_err("missing sections");
    assert!(
        error.to_string().contains("When to use") || error.to_string().contains("sections"),
        "{error}"
    );
}
