use std::sync::Arc;

use hearth_core::{Harness, Session};
use hearth_tools::Tool;
use hearth_memory::{
    MemoryError, MemoryKind, MemoryPatch, MemoryService, MemoryStatus, MemoryTool, NewMemoryInput,
    SourceKind,
};
use serde_json::json;
use tempfile::TempDir;

fn setup() -> (TempDir, Session, MemoryService) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session().clone();
    let service = MemoryService::new(session.clone());
    (home, session, service)
}

fn input(content: &str, kind: MemoryKind) -> NewMemoryInput {
    NewMemoryInput {
        content: content.to_owned(),
        kind,
        tags: Vec::new(),
        importance: 50,
        source_kind: SourceKind::User,
        source_ref: None,
        supersedes: None,
        valid_from: None,
        valid_to: None,
    }
}

fn count(session: &Session, sql: &str) -> i64 {
    session
        .storage()
        .read(|reader| reader.query_row(sql, [], |row| row.get(0)))
        .expect("count")
}

#[test]
fn a_memory_can_be_saved_and_read_back() {
    let (_home, _session, service) = setup();
    let saved = service
        .save(input("Sam prefers dark roast coffee", MemoryKind::Preference))
        .expect("save");
    assert_eq!(saved.content, "Sam prefers dark roast coffee");
    assert_eq!(saved.kind, MemoryKind::Preference);
    assert_eq!(saved.status, MemoryStatus::Active);
    assert!(saved.fingerprint.len() > 10);

    let read = service.get(&saved.id).expect("get").expect("exists");
    assert_eq!(read.id, saved.id);
    assert_eq!(read.content, saved.content);
}

#[test]
fn an_exact_duplicate_merges_instead_of_adding_a_row() {
    let (_home, _session, service) = setup();
    let first = service
        .save(input("Sam prefers dark roast coffee", MemoryKind::Preference))
        .expect("first");
    let mut second_input = input("Sam prefers dark roast coffee", MemoryKind::Preference);
    second_input.importance = 80;
    second_input.tags = vec!["coffee".to_owned()];
    let second = service.save(second_input).expect("second");

    assert_eq!(second.id, first.id);
    assert_eq!(count(&_session, "SELECT COUNT(*) FROM memory_items"), 1);
    assert_eq!(second.importance, 80);
    assert_eq!(second.tags, vec!["coffee".to_owned()]);
}

#[test]
fn a_near_duplicate_merges() {
    let (_home, _session, service) = setup();
    let first = service
        .save(input("The patient prefers dark roast coffee", MemoryKind::Preference))
        .expect("first");
    let second = service
        .save(input(
            "The patient prefers dark roast coffee daily",
            MemoryKind::Preference,
        ))
        .expect("second");
    assert_eq!(second.id, first.id);
    assert_eq!(count(&_session, "SELECT COUNT(*) FROM memory_items"), 1);
}

#[test]
fn a_superseding_memory_closes_the_old_one() {
    let (_home, _session, service) = setup();
    let old = service
        .save(input("Sam lives in Manila", MemoryKind::Fact))
        .expect("old");
    let mut next = input("Sam lives in Quezon City", MemoryKind::Fact);
    next.supersedes = Some(old.id.clone());
    let new = service.save(next).expect("new");

    let old = service.get(&old.id).expect("get").expect("old");
    let new = service.get(&new.id).expect("get").expect("new");
    assert_eq!(old.status, MemoryStatus::Superseded);
    assert!(old.valid_to.is_some());
    assert_eq!(new.supersedes_id.as_deref(), Some(old.id.as_str()));
    assert_eq!(new.status, MemoryStatus::Active);
}

#[test]
fn an_update_changes_content_and_reindexes_search() {
    let (_home, _session, service) = setup();
    let saved = service
        .save(input("Sam likes tea", MemoryKind::Preference))
        .expect("save");
    let updated = service
        .update(
            &saved.id,
            MemoryPatch {
                content: Some("Sam likes coffee".to_owned()),
                importance: Some(90),
                ..MemoryPatch::default()
            },
        )
        .expect("update");
    assert_eq!(updated.content, "Sam likes coffee");
    assert_eq!(updated.importance, 90);
    assert!(service.search("coffee", 10).expect("search").iter().any(|item| item.id == saved.id));
    assert!(!service.search("tea", 10).expect("search").iter().any(|item| item.id == saved.id));
}

#[test]
fn forget_keeps_a_tombstone_and_removes_search() {
    let (_home, _session, service) = setup();
    let saved = service
        .save(input("Sam likes coffee", MemoryKind::Preference))
        .expect("save");
    service.forget(&saved.id).expect("forget");
    let item = service.get(&saved.id).expect("get").expect("tombstone");
    assert_eq!(item.status, MemoryStatus::Deleted);
    assert!(service.search("coffee", 10).expect("search").is_empty());
    assert_eq!(
        _session
            .storage()
            .read(|reader| reader.query_row("SELECT COUNT(*) FROM memory_items", [], |row| row.get(0)))
            .expect("count"),
        1
    );
}

#[test]
fn pin_sets_and_clears_standing() {
    let (_home, _session, service) = setup();
    let saved = service
        .save(input("Sam's timezone is Asia/Manila", MemoryKind::Rule))
        .expect("save");
    let pinned = service.pin(&saved.id, true).expect("pin");
    assert_eq!(pinned.standing, hearth_memory::Standing::Pinned);
    let unpinned = service.pin(&saved.id, false).expect("unpin");
    assert_eq!(unpinned.standing, hearth_memory::Standing::None);
}

#[test]
fn search_finds_by_tag_and_returns_stats() {
    let (_home, _session, service) = setup();
    let mut tagged = input("Sam's clinic is in Makati", MemoryKind::Project);
    tagged.tags = vec!["clinic".to_owned(), "location".to_owned()];
    service.save(tagged).expect("save");
    let hits = service.search("clinic", 10).expect("search");
    assert_eq!(hits.len(), 1);
    let stats = service.stats().expect("stats");
    assert_eq!(stats.total, 1);
    assert_eq!(stats.by_kind.get("project"), Some(&1));
}

#[test]
fn as_of_reads_the_state_at_a_time() {
    let (_home, session, service) = setup();
    let now = session.now_micros();
    let mut old = input("Sam lives in Manila", MemoryKind::Fact);
    old.valid_from = Some(now - 10_000_000);
    let old = service.save(old).expect("old");
    let between = session.now_micros();
    let mut next = input("Sam lives in Quezon City", MemoryKind::Fact);
    next.supersedes = Some(old.id.clone());
    next.valid_from = Some(session.now_micros());
    service.save(next).expect("new");

    let past = service
        .as_of(None, between)
        .expect("as of past");
    assert_eq!(past.len(), 1);
    assert_eq!(past[0].content, "Sam lives in Manila");
}

#[test]
fn secrets_are_refused() {
    let (_home, _session, service) = setup();
    for content in [
        "my key is sk-live-abcdefghijklmnop",
        "token=abcdefgh1234",
        "123456789:AAFakeTokenFakeTokenFakeTokenFake",
    ] {
        let error = service
            .save(input(content, MemoryKind::Fact))
            .expect_err("secret must be refused");
        assert!(matches!(error, MemoryError::Secret(_)));
    }
}

#[test]
fn memory_events_record_the_actor_and_actions() {
    let (_home, session, service) = setup();
    let saved = service
        .save(input("Sam likes coffee", MemoryKind::Preference))
        .expect("save");
    service
        .update(
            &saved.id,
            MemoryPatch {
                importance: Some(75),
                ..MemoryPatch::default()
            },
        )
        .expect("update");
    service.forget(&saved.id).expect("forget");
    let actions: Vec<String> = session
        .storage()
        .read(|reader| {
            reader.query("SELECT action FROM memory_events ORDER BY created_at, id", [], |row| {
                row.get(0)
            })
        })
        .expect("events");
    assert_eq!(actions, vec!["add", "update", "forget"]);
}

#[test]
fn the_memory_tool_covers_every_action() {
    let (_home, _session, service) = setup();
    let tool = MemoryTool::new(Arc::new(service));
    let saved = tool
        .call(&json!({"action": "save", "content": "Sam likes coffee", "kind": "preference", "importance": 70, "tags": ["coffee"]}))
        .expect("save");
    let id = saved.content.trim().to_owned();
    assert!(id.starts_with("01"));

    let search = tool
        .call(&json!({"action": "search", "query": "coffee"}))
        .expect("search");
    assert!(search.content.contains("Sam likes coffee"));

    let get = tool
        .call(&json!({"action": "get", "id": id}))
        .expect("get");
    assert!(get.content.contains("Sam likes coffee"));

    let update = tool
        .call(&json!({"action": "update", "id": id, "importance": 90}))
        .expect("update");
    assert!(update.content.contains("updated"));

    let pin = tool
        .call(&json!({"action": "pin", "id": id}))
        .expect("pin");
    assert!(pin.content.contains("pinned"));

    let stats = tool
        .call(&json!({"action": "stats"}))
        .expect("stats");
    assert!(stats.content.contains("\"total\""));

    let as_of = tool
        .call(&json!({"action": "as_of", "at": "2030-01-01"}))
        .expect("as of");
    assert!(as_of.content.contains("Sam likes coffee"));

    let forget = tool
        .call(&json!({"action": "forget", "id": id}))
        .expect("forget");
    assert!(forget.content.contains("forgotten"));
}
