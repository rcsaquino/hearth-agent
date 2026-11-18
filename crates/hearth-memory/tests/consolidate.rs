//! S4: the deterministic consolidation pass and its report.

use hearth_core::{Harness, Session};
use hearth_memory::{ConsolidationReport, Kind, MemoryService, NewMemory};
use hearth_storage::rusqlite::params;
use tempfile::TempDir;

fn home() -> (TempDir, Harness) {
    let directory = TempDir::new().expect("temp home");
    let harness = Harness::open(directory.path()).expect("harness");
    (directory, harness)
}

fn seed(session: &Session, id: &str, content: &str, subject: &str, kind: &str, pinned: bool) {
    let now = session.now_micros();
    let fingerprint = hearth_memory::fingerprint(content);
    let id = id.to_owned();
    let content = content.to_owned();
    let subject = subject.to_owned();
    let kind = kind.to_owned();
    session
        .storage()
        .write(move |writer| {
            writer.execute(
                "INSERT INTO memories (id, kind, content, subject, terms, fingerprint, pinned, \
                 source_kind, source_entry, valid_from, valid_to, created_at, updated_at, revision) \
                 VALUES (?, ?, ?, ?, '', ?, ?, 'user', NULL, ?, NULL, ?, ?, 1)",
                params![
                    id,
                    kind,
                    content,
                    subject,
                    fingerprint,
                    i64::from(pinned),
                    now,
                    now,
                    now
                ],
            )?;
            Ok::<_, hearth_memory::MemoryError>(())
        })
        .expect("seed row");
}

fn current(service: &MemoryService, id: &str) -> hearth_memory::Memory {
    service
        .get(id)
        .expect("get")
        .unwrap_or_else(|| panic!("missing {id}"))
}

#[test]
fn a_canonical_duplicate_closes_all_but_one() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_dup1",
        "the user prefers dark mode",
        "user",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_dup2",
        "the user prefers dark mode",
        "user",
        "preference",
        false,
    );

    let report = service.consolidate().expect("consolidate");
    assert_eq!(report.canonical_closed, 1, "{report:?}");
    let first = current(&service, "m_dup1");
    let second = current(&service, "m_dup2");
    assert!(
        first.valid_to.is_some() || second.valid_to.is_some(),
        "one row must close"
    );
    assert!(
        first.valid_to.is_none() || second.valid_to.is_none(),
        "one row must stay current"
    );
    let (kept, closed) = if first.valid_to.is_none() {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(closed.superseded_by.as_deref(), Some(kept.id.as_str()));
    assert_eq!(service.count().expect("count"), 1);
}

#[test]
fn a_near_duplicate_the_write_gate_refuses_is_closed_by_the_pass() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_near1",
        "the user prefers dark mode",
        "user",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_near2",
        "the user prefers dark mode please",
        "user",
        "preference",
        false,
    );

    let report = service.consolidate().expect("consolidate");
    assert_eq!(report.near_closed, 1, "{report:?}");
    assert_eq!(report.near_duplicates, 0, "a closed pair is not reported");
    let first = current(&service, "m_near1");
    let second = current(&service, "m_near2");
    assert!(
        first.valid_to.is_some() || second.valid_to.is_some(),
        "one row must close"
    );
    let (kept, closed) = if first.valid_to.is_none() {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(closed.superseded_by.as_deref(), Some(kept.id.as_str()));
}

#[test]
fn a_near_pair_keeps_the_fuller_wording() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_full",
        "kya has a nut allergy including pistachio and carries an epipen",
        "kya",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_thin",
        "kya has a nut allergy",
        "kya",
        "fact",
        false,
    );

    service.consolidate().expect("consolidate");
    assert!(
        current(&service, "m_full").valid_to.is_none(),
        "the fuller wording must stay current even when it is older"
    );
    assert_eq!(
        current(&service, "m_thin").superseded_by.as_deref(),
        Some("m_full")
    );
}

#[test]
fn two_different_facts_about_one_subject_are_left_current() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_allergy",
        "kya has a nut allergy",
        "kya",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_sister",
        "kya's sister is ana",
        "kya",
        "person",
        false,
    );

    let report = service.consolidate().expect("consolidate");
    assert_eq!(report.near_duplicates, 0, "{report:?}");
    assert!(current(&service, "m_allergy").valid_to.is_none());
    assert!(current(&service, "m_sister").valid_to.is_none());
}

#[test]
fn compound_rows_are_reported_and_not_split() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    let content = "the user likes apples. He buys one every day.";
    seed(
        harness.session(),
        "m_compound",
        content,
        "user",
        "fact",
        false,
    );

    let report = service.consolidate().expect("consolidate");
    assert!(report.compound >= 1, "{report:?}");
    let row = current(&service, "m_compound");
    assert!(row.valid_to.is_none());
    assert_eq!(row.content, content, "the pass must not rewrite content");
    assert!(
        report
            .compound_rows
            .iter()
            .any(|candidate| candidate.id == "m_compound"),
        "{report:?}"
    );
}

#[test]
fn the_same_content_under_two_subjects_is_reported_and_left_current() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_alice",
        "the release gate needs two approvals",
        "alice",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_bob",
        "the release gate needs two approvals",
        "bob",
        "fact",
        false,
    );

    let report = service.consolidate().expect("consolidate");
    assert!(report.cross_subject >= 1, "{report:?}");
    assert!(current(&service, "m_alice").valid_to.is_none());
    assert!(current(&service, "m_bob").valid_to.is_none());
}

#[test]
fn a_second_run_changes_nothing() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_dup1",
        "the user prefers dark mode",
        "user",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_dup2",
        "the user prefers dark mode",
        "user",
        "preference",
        false,
    );
    let first = service.consolidate().expect("first run");
    assert_eq!(first.canonical_closed, 1);
    let second = service.consolidate().expect("second run");
    assert_eq!(second.canonical_closed, 0, "{second:?}");
    assert_eq!(second.near_duplicates, 0, "{second:?}");
    assert_eq!(second.cross_subject, 0, "{second:?}");
    assert_eq!(second.compound, 0, "{second:?}");
    assert_eq!(service.count().expect("count"), 1);
}

#[test]
fn the_report_is_stamped_beside_the_repair() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_dup1",
        "the user prefers dark mode",
        "user",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_dup2",
        "the user prefers dark mode",
        "user",
        "preference",
        false,
    );
    let report = service.consolidate().expect("consolidate");
    let session = harness.session();
    let stamp = session
        .meta_value("last_consolidation")
        .expect("stamp lookup")
        .expect("a stamp");
    assert!(stamp.contains('T'), "the stamp is an instant: {stamp}");
    let stored: ConsolidationReport = serde_json::from_str(
        &session
            .meta_value("last_consolidation_report")
            .expect("report lookup")
            .expect("a report"),
    )
    .expect("decode report");
    assert_eq!(stored.canonical_closed, report.canonical_closed);
}

#[test]
fn a_duplicate_created_before_the_pass_is_repaired_by_it() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    service
        .write(NewMemory::new("the user likes tea").subject("user"))
        .expect("first");
    seed(
        harness.session(),
        "m_late_duplicate",
        "the user likes tea",
        "user",
        "preference",
        false,
    );
    assert_eq!(service.count().expect("count"), 2);
    let report = service.consolidate().expect("consolidate");
    assert_eq!(report.canonical_closed, 1);
    assert_eq!(service.count().expect("count"), 1);
}

#[test]
fn a_pinned_rule_is_the_survivor_of_a_canonical_group() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_rule",
        "keep replies short",
        "",
        "rule",
        true,
    );
    seed(
        harness.session(),
        "m_fact",
        "keep replies short",
        "",
        "fact",
        false,
    );
    let report = service.consolidate().expect("consolidate");
    assert_eq!(report.canonical_closed, 1);
    let rule = current(&service, "m_rule");
    assert!(rule.valid_to.is_none(), "the pinned rule must survive");
    assert!(rule.pinned);
    assert_eq!(
        current(&service, "m_fact").superseded_by.as_deref(),
        Some("m_rule")
    );
}

#[test]
fn the_kind_prior_decides_between_two_general_rows() {
    let (_directory, harness) = home();
    let service = MemoryService::new(harness.session().clone());
    seed(
        harness.session(),
        "m_fact",
        "the user keeps a notebook",
        "user",
        "fact",
        false,
    );
    seed(
        harness.session(),
        "m_pref",
        "the user keeps a notebook",
        "user",
        "preference",
        false,
    );
    service.consolidate().expect("consolidate");
    assert_eq!(current(&service, "m_fact").kind, Kind::Fact);
    assert_eq!(current(&service, "m_pref").kind, Kind::Preference);
    assert!(current(&service, "m_fact").valid_to.is_some());
    assert!(current(&service, "m_pref").valid_to.is_none());
}
