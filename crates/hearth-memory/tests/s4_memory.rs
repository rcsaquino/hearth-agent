use std::sync::Arc;

use hearth_core::{Harness, MemoryChange, MemoryWrite, NewMemory, Session};
use hearth_memory::{
    MemoryError, MemoryKind, MemoryPatch, MemoryService, MemoryStatus, MemoryTool, NewMemoryInput,
    SourceKind,
};
use hearth_tools::Tool;
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
        .save(input(
            "Sam prefers dark roast coffee",
            MemoryKind::Preference,
        ))
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
        .save(input(
            "Sam prefers dark roast coffee",
            MemoryKind::Preference,
        ))
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
        .save(input(
            "The patient prefers dark roast coffee",
            MemoryKind::Preference,
        ))
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
    assert!(
        service
            .search("coffee", 10)
            .expect("search")
            .iter()
            .any(|item| item.id == saved.id)
    );
    assert!(
        !service
            .search("tea", 10)
            .expect("search")
            .iter()
            .any(|item| item.id == saved.id)
    );
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
    assert_eq!(count(&_session, "SELECT COUNT(*) FROM memory_items"), 1);
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

    let past = service.as_of(None, between).expect("as of past");
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
        "authorization is Bearer abcdefghijklmnop",
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
            reader.query(
                "SELECT action FROM memory_events ORDER BY created_at, id",
                [],
                |row| row.get(0),
            )
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

    let get = tool.call(&json!({"action": "get", "id": id})).expect("get");
    assert!(get.content.contains("Sam likes coffee"));

    let update = tool
        .call(&json!({"action": "update", "id": id, "importance": 90}))
        .expect("update");
    assert!(update.content.contains("updated"));

    let pin = tool.call(&json!({"action": "pin", "id": id})).expect("pin");
    assert!(pin.content.contains("pinned"));

    let stats = tool.call(&json!({"action": "stats"})).expect("stats");
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

// ---- M4b: standing memory and automatic recall ----

use hearth_core::{Clock, IdGen, UuidV7};
use hearth_memory::{MemoryPolicy, Standing};
use std::sync::atomic::{AtomicI64, Ordering};

struct StepClock(AtomicI64);

impl StepClock {
    fn new() -> Self {
        Self(AtomicI64::new(1_700_000_000_000_000))
    }

    fn advance_days(&self, days: i64) {
        self.0.fetch_add(days * 86_400_000_000, Ordering::Relaxed);
    }
}

impl Clock for StepClock {
    fn now_micros(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

fn setup_clock(policy: MemoryPolicy) -> (TempDir, Arc<StepClock>, Session, MemoryService) {
    let home = TempDir::new().expect("temp home");
    let clock = Arc::new(StepClock::new());
    let harness = Harness::open_with(
        home.path(),
        clock.clone(),
        Arc::new(UuidV7) as Arc<dyn IdGen>,
    )
    .expect("open harness");
    let session = harness.session().clone();
    let service = MemoryService::with_policy(session.clone(), policy);
    (home, clock, session, service)
}

fn input_imp(content: &str, kind: MemoryKind, importance: i64) -> NewMemoryInput {
    NewMemoryInput {
        content: content.to_owned(),
        kind,
        tags: Vec::new(),
        importance,
        source_kind: SourceKind::User,
        source_ref: None,
        supersedes: None,
        valid_from: None,
        valid_to: None,
    }
}

#[test]
fn a_high_importance_rule_promotes_into_standing_and_bumps_the_version() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    let saved = service
        .save(input_imp("Use metric units", MemoryKind::Rule, 90))
        .expect("save");
    assert_eq!(saved.standing, Standing::Promoted);

    let standing = service.standing().expect("standing");
    assert_eq!(standing.items.len(), 1);
    assert_eq!(standing.items[0].id, saved.id);
    assert_eq!(standing.version, 1);
    assert_eq!(service.standing_version().expect("version"), 1);
    assert!(
        service
            .render_standing(&standing.items)
            .contains("Use metric units")
    );
}

#[test]
fn a_low_importance_fact_does_not_change_standing() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    service
        .save(input_imp("Sam likes blue pens", MemoryKind::Fact, 50))
        .expect("save");
    let standing = service.standing().expect("standing");
    assert!(standing.items.is_empty());
    assert_eq!(standing.version, 0);
}

#[test]
fn pinned_items_are_always_standing() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    let saved = service
        .save(input_imp("Sam likes blue pens", MemoryKind::Fact, 50))
        .expect("save");
    let pinned = service.pin(&saved.id, true).expect("pin");
    assert_eq!(pinned.standing, Standing::Pinned);
    let standing = service.standing().expect("standing");
    assert_eq!(standing.items.len(), 1);
    assert_eq!(standing.items[0].id, saved.id);
    assert_eq!(standing.version, 1);
}

#[test]
fn promoted_items_over_the_cap_are_demoted() {
    let policy = MemoryPolicy {
        standing_max_tokens: 35,
        ..MemoryPolicy::default()
    };
    let (_home, _clock, _session, service) = setup_clock(policy);
    let first = service
        .save(input_imp(
            "Rule one: always answer in metric units and never guess",
            MemoryKind::Rule,
            90,
        ))
        .expect("first");
    let second = service
        .save(input_imp(
            "Rule two: prefer short answers and avoid long explanations",
            MemoryKind::Rule,
            90,
        ))
        .expect("second");
    let third = service
        .save(input_imp(
            "Rule three: check the time before scheduling anything",
            MemoryKind::Rule,
            90,
        ))
        .expect("third");
    let standing = service.standing().expect("standing");
    assert!(standing.items.len() < 3);
    assert_eq!(standing.items[0].id, first.id);
    assert_eq!(standing.items[0].standing, Standing::Promoted);
    let mut demoted = 0;
    for id in [&second.id, &third.id] {
        let item = service.get(id).expect("get").expect("exists");
        if item.standing == Standing::None {
            demoted += 1;
        }
    }
    assert!(demoted >= 1);
    assert!(standing.version >= 1);
}

#[test]
fn the_standing_version_changes_only_when_the_set_changes() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    let rule = service
        .save(input_imp("Use metric units", MemoryKind::Rule, 90))
        .expect("rule");
    assert_eq!(service.standing_version().expect("v"), 1);
    let fact = service
        .save(input_imp("Sam likes blue pens", MemoryKind::Fact, 50))
        .expect("fact");
    assert_eq!(service.standing_version().expect("v"), 1);
    service
        .update(
            &rule.id,
            MemoryPatch {
                content: Some("Use metric units and Celsius".to_owned()),
                ..MemoryPatch::default()
            },
        )
        .expect("update rule");
    assert_eq!(service.standing_version().expect("v"), 2);
    service
        .update(
            &fact.id,
            MemoryPatch {
                content: Some("Sam likes green pens".to_owned()),
                ..MemoryPatch::default()
            },
        )
        .expect("update fact");
    assert_eq!(service.standing_version().expect("v"), 2);
    service.pin(&fact.id, true).expect("pin");
    assert_eq!(service.standing_version().expect("v"), 3);
    service.forget(&rule.id).expect("forget");
    assert_eq!(service.standing_version().expect("v"), 4);
}

#[test]
fn recall_ranks_a_recent_important_match_above_an_old_weak_one() {
    let (_home, clock, _session, service) = setup_clock(MemoryPolicy::default());
    let old = service
        .save(input_imp(
            "coffee preference old",
            MemoryKind::Preference,
            20,
        ))
        .expect("old");
    clock.advance_days(365);
    let recent = service
        .save(input_imp(
            "coffee preference recent",
            MemoryKind::Preference,
            90,
        ))
        .expect("recent");
    let recalled = service.recall("coffee").expect("recall");
    assert_eq!(recalled[0].id, recent.id);
    let old_pos = recalled.iter().position(|item| item.id == old.id);
    assert!(old_pos.is_none() || old_pos == Some(recalled.len() - 1));
    let read = service.get(&recent.id).expect("get").expect("exists");
    assert!(read.last_used_at.is_some());
}

#[test]
fn recall_caps_items_and_budget() {
    let policy = MemoryPolicy {
        recall_max_tokens: 40,
        ..MemoryPolicy::default()
    };
    let (_home, _clock, _session, service) = setup_clock(policy);
    for word in [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
    ] {
        service
            .save(input_imp(
                &format!("coffee note {word}"),
                MemoryKind::Fact,
                50,
            ))
            .expect("save");
    }
    let recalled = service.recall("coffee").expect("recall");
    assert!(recalled.len() <= 6);
    let tokens: i64 = recalled
        .iter()
        .map(|item| hearth_model::estimate_tokens(&service.render_line(item)))
        .sum();
    assert!(tokens <= 40, "recall used {tokens} tokens");
}

#[test]
fn recall_keeps_at_most_two_per_kind() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    for word in ["alpha", "bravo", "charlie", "delta"] {
        service
            .save(input_imp(
                &format!("coffee fact {word}"),
                MemoryKind::Fact,
                60,
            ))
            .expect("fact");
        service
            .save(input_imp(
                &format!("coffee preference {word}"),
                MemoryKind::Preference,
                60,
            ))
            .expect("preference");
    }
    let recalled = service.recall("coffee").expect("recall");
    let facts = recalled
        .iter()
        .filter(|item| item.kind == MemoryKind::Fact)
        .count();
    let prefs = recalled
        .iter()
        .filter(|item| item.kind == MemoryKind::Preference)
        .count();
    assert!(facts <= 2, "facts {facts}");
    assert!(prefs <= 2, "prefs {prefs}");
}

#[test]
fn trivial_input_returns_no_recall() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    service
        .save(input_imp("coffee note alpha", MemoryKind::Fact, 60))
        .expect("save");
    for query in ["", "hi", "thanks", "ok", "👍", "a"] {
        assert!(
            service.recall(query).expect("recall").is_empty(),
            "query {query:?} returned recall"
        );
    }
}

#[test]
fn recall_finds_a_memory_by_tag() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    let mut tagged = input_imp("Sam drinks espresso", MemoryKind::Preference, 60);
    tagged.tags = vec!["coffee".to_owned()];
    let saved = service.save(tagged).expect("save");
    let recalled = service.recall("coffee").expect("recall");
    assert!(recalled.iter().any(|item| item.id == saved.id));
}

#[test]
fn recall_includes_standing_items() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    let pinned = service
        .save(input_imp("Use metric units", MemoryKind::Rule, 50))
        .expect("save");
    service.pin(&pinned.id, true).expect("pin");
    service
        .save(input_imp("coffee note alpha", MemoryKind::Fact, 60))
        .expect("save");
    let recalled = service.recall("coffee").expect("recall");
    assert!(recalled.iter().any(|item| item.id == pinned.id));
}

#[test]
fn the_standing_render_includes_id_kind_date_and_importance() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    let saved = service
        .save(input_imp("Use metric units", MemoryKind::Rule, 90))
        .expect("save");
    let standing = service.standing().expect("standing");
    let rendered = service.render_standing(&standing.items);
    assert!(rendered.contains(&saved.id));
    assert!(rendered.contains("rule"));
    assert!(rendered.contains("p=90"));
    assert!(rendered.contains("Use metric units"));
}

// ---- M4c: standing refresh and consolidation ----

fn raw_memory(
    session: &Session,
    content: &str,
    kind: MemoryKind,
    importance: i64,
    fingerprint: &str,
    tags: &[&str],
) -> String {
    let id = session.new_id();
    let now = session.now_micros();
    let item = NewMemory {
        id: id.clone(),
        kind: kind.as_str().to_owned(),
        content: content.to_owned(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        fingerprint: fingerprint.to_owned(),
        importance,
        status: MemoryStatus::Active.as_str().to_owned(),
        valid_from: None,
        valid_to: None,
        supersedes_id: None,
        source_kind: SourceKind::User.as_str().to_owned(),
        source_ref: None,
        standing: "none".to_owned(),
        created_at: now,
        last_used_at: None,
    };
    session
        .save_memory(vec![MemoryWrite::new(MemoryChange::Add(item), "system")])
        .expect("raw memory");
    id
}

#[test]
fn a_standing_refresh_promotes_a_qualifying_row_and_bumps_the_version() {
    let (_home, _clock, session, service) = setup_clock(MemoryPolicy::default());
    let id = raw_memory(
        &session,
        "Use metric units",
        MemoryKind::Rule,
        90,
        "fp-refresh-promote",
        &[],
    );
    assert_eq!(service.standing_version().expect("v"), 0);
    let standing = service.standing().expect("standing");
    assert_eq!(standing.items.len(), 1);
    assert_eq!(standing.items[0].id, id);
    assert_eq!(standing.items[0].standing, Standing::Promoted);
    assert_eq!(standing.version, 1);
    assert_eq!(service.standing_version().expect("v"), 1);
    let saved = service.get(&id).expect("get").expect("row");
    assert_eq!(saved.standing, Standing::Promoted);
}

#[test]
fn a_qualifying_row_demoted_by_the_cap_leaves_the_version_unchanged() {
    let policy = MemoryPolicy {
        standing_max_tokens: 0,
        ..MemoryPolicy::default()
    };
    let (_home, _clock, session, service) = setup_clock(policy);
    let id = raw_memory(
        &session,
        "Use metric units",
        MemoryKind::Rule,
        90,
        "fp-cap-demote",
        &[],
    );
    assert_eq!(service.standing_version().expect("v"), 0);
    let standing = service.standing().expect("standing");
    assert!(standing.items.is_empty());
    assert_eq!(standing.version, 0);
    assert_eq!(service.standing_version().expect("v"), 0);
    let saved = service.get(&id).expect("get").expect("row");
    assert_eq!(saved.standing, Standing::None);
}

#[test]
fn near_duplicates_consolidate_into_one_active_row() {
    let (_home, _clock, session, service) = setup_clock(MemoryPolicy::default());
    let survivor = raw_memory(
        &session,
        "The patient prefers dark roast coffee",
        MemoryKind::Preference,
        50,
        "fp-consolidate-a",
        &[],
    );
    let duplicate = raw_memory(
        &session,
        "The patient prefers dark roast coffee daily",
        MemoryKind::Preference,
        70,
        "fp-consolidate-b",
        &["coffee"],
    );
    let unrelated = raw_memory(
        &session,
        "Sam lives in Manila",
        MemoryKind::Fact,
        50,
        "fp-consolidate-c",
        &[],
    );

    let report = service.consolidate().expect("consolidate");
    assert_eq!(report.merged, 1);
    assert_eq!(report.unchanged, 2);

    let survivor_item = service.get(&survivor).expect("get").expect("survivor");
    let duplicate_item = service.get(&duplicate).expect("get").expect("duplicate");
    let unrelated_item = service.get(&unrelated).expect("get").expect("unrelated");
    assert_eq!(survivor_item.status, MemoryStatus::Active);
    assert_eq!(survivor_item.importance, 70);
    assert!(survivor_item.tags.contains(&"coffee".to_owned()));
    assert_eq!(duplicate_item.status, MemoryStatus::Superseded);
    assert!(duplicate_item.valid_to.is_some());
    assert_eq!(unrelated_item.status, MemoryStatus::Active);

    let events = count(
        &session,
        "SELECT COUNT(*) FROM memory_events WHERE action = 'update'",
    );
    assert!(events >= 2, "merge events {events}");

    let active = service.search("coffee", 10).expect("search");
    assert!(active.iter().any(|item| item.id == survivor));
    assert!(!active.iter().any(|item| item.id == duplicate));
}

#[test]
fn a_direct_fact_is_recalled_when_standing_memory_is_full() {
    let (_home, _session, service) = setup();
    for index in 0..6 {
        let mut preference = input(
            &format!("Sam prefers style {index}"),
            MemoryKind::Preference,
        );
        preference.importance = 95;
        service.save(preference).expect("save preference");
    }
    let fact = service
        .save(input(
            "The primary project is Project Atlas.",
            MemoryKind::Fact,
        ))
        .expect("save fact");
    let items = service
        .recall("What is the primary project?")
        .expect("recall");
    assert!(
        items.iter().any(|item| item.id == fact.id),
        "direct fact was pushed out by standing memory: {:?}",
        items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn as_of_returns_a_fact_learned_after_the_query_time() {
    let (_home, session, service) = setup();
    let now = session.now_micros();
    let mut fact = input("The lab is in Building 4.", MemoryKind::Fact);
    fact.valid_from = Some(now - 10_000_000);
    let saved = service.save(fact).expect("save");
    let items = service
        .as_of(Some("Where is the lab?"), now - 5_000_000)
        .expect("as_of");
    assert!(
        items.iter().any(|item| item.id == saved.id),
        "as_of missed a fact that was true before it was learned"
    );
}

#[test]
fn distinct_numeric_versions_are_not_merged() {
    let (_home, _session, service) = setup();
    let first = service
        .save(input("The tool version is 1.0.", MemoryKind::Fact))
        .expect("first");
    let second = service
        .save(input("The tool version is 1.1.", MemoryKind::Fact))
        .expect("second");
    assert_ne!(first.id, second.id, "distinct versions were merged");
    assert_eq!(service.stats().expect("stats").total, 2);
}

#[test]
fn a_tag_only_match_is_recalled() {
    let (_home, _session, service) = setup();
    let mut memory = input("The spare drive is ready.", MemoryKind::Fact);
    memory.tags = vec!["nebula".to_owned()];
    let saved = service.save(memory).expect("save");
    let items = service.recall("nebula").expect("recall");
    assert!(items.iter().any(|item| item.id == saved.id));
}

#[test]
fn common_question_words_do_not_flood_recall() {
    let (_home, _session, service) = setup();
    for index in 0..10 {
        service
            .save(input(
                &format!("The note {index} is about the weather."),
                MemoryKind::Fact,
            ))
            .expect("save noise");
    }
    let fact = service
        .save(input(
            "The primary project is Project Atlas.",
            MemoryKind::Fact,
        ))
        .expect("save fact");
    let items = service
        .recall("What is the primary project?")
        .expect("recall");
    assert!(
        items.iter().any(|item| item.id == fact.id),
        "stopword-only matches flooded recall: {:?}",
        items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_fixture_direct_question_returns_the_expected_fact() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../hearth-eval/fixtures/memory_bench.json"))
            .expect("fixture");
    let (_home, _session, service) = setup();
    let mut keys = std::collections::BTreeMap::new();
    for memory in fixture["memories"].as_array().expect("memories") {
        let key = memory["key"].as_str().expect("key").to_owned();
        let kind = MemoryKind::parse(memory["kind"].as_str().expect("kind")).expect("kind");
        let tags = memory["tags"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let supersedes = memory["supersedes_key"]
            .as_str()
            .and_then(|old| keys.get(old).cloned());
        let input = NewMemoryInput {
            content: memory["content"].as_str().expect("content").to_owned(),
            kind,
            tags,
            importance: memory["importance"].as_i64().unwrap_or(50),
            source_kind: SourceKind::Session,
            source_ref: Some(key.clone()),
            supersedes,
            valid_from: memory["valid_from"].as_i64(),
            valid_to: memory["valid_to"].as_i64(),
        };
        let item = service.save(input).expect("save fixture memory");
        keys.insert(key, item.id);
    }
    let expected = keys.get("m001").expect("m001").clone();
    let items = service
        .recall("What is the primary project?")
        .expect("recall");
    assert!(
        items.iter().any(|item| item.id == expected),
        "fixture direct question missed m001: {:?}",
        items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn unrelated_standing_preferences_do_not_crowd_a_query_specific_one() {
    let (_home, _clock, _session, service) = setup_clock(MemoryPolicy::default());
    service
        .save(input_imp("Prefers short replies.", MemoryKind::Preference, 90))
        .expect("reply preference");
    service
        .save(input_imp("Uses metric units.", MemoryKind::Preference, 90))
        .expect("unit preference");
    let meetings = service
        .save(input_imp(
            "Dislikes meetings before 10.",
            MemoryKind::Preference,
            64,
        ))
        .expect("meeting preference");
    let recalled = service.recall("When are meetings disliked?").expect("recall");
    assert!(
        recalled.iter().any(|item| item.id == meetings.id),
        "query-specific preference was crowded out: {recalled:#?}"
    );
}
