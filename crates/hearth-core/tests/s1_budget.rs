use hearth_core::{ContextBudget, TokenCounts};

#[test]
fn the_default_budget_matches_the_spec() {
    let budget = ContextBudget::default();
    assert_eq!(budget.context_window, 128_000);
    assert_eq!(budget.max_output_tokens, 32_000);
    assert_eq!(budget.reserve, 33_024);
    assert_eq!(budget.usable, 94_976);
    assert_eq!(budget.soft, 61_734);
    assert_eq!(budget.hard, 80_729);
    assert_eq!(budget.keep_recent, 20_000);
}

#[test]
fn a_small_window_is_calculated() {
    let budget = ContextBudget::new(10_000, 2_000, 0.65, 0.85, 24_000);
    assert_eq!(budget.reserve, 5_120);
    assert_eq!(budget.usable, 4_880);
    assert_eq!(budget.soft, 3_172);
    assert_eq!(budget.hard, 4_148);
    assert_eq!(budget.keep_recent, 1_708);
}

#[test]
fn a_window_smaller_than_the_reserve_has_no_usable_space() {
    let budget = ContextBudget::new(4_096, 8_192, 0.65, 0.85, 24_000);
    assert_eq!(budget.usable, 0);
    assert_eq!(budget.soft, 0);
    assert_eq!(budget.hard, 0);
    assert_eq!(budget.keep_recent, 0);
}

#[test]
fn the_keep_budget_is_capped_at_thirty_five_percent() {
    let budget = ContextBudget::new(128_000, 8_192, 0.65, 0.85, 10_000);
    assert_eq!(budget.keep_recent, 10_000);
}

#[test]
fn token_counts_sum_and_report_the_fixed_overhead() {
    let counts = TokenCounts::new(10, 30, 40, 0, 0, 50, 60, 70);
    assert_eq!(counts.total, 260);
    assert_eq!(counts.fixed, 150);
}

#[test]
fn an_image_carries_its_provider_cost() {
    use hearth_core::{Harness, Save};
    use serde_json::json;
    use tempfile::TempDir;

    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Photo", "local", "local:photo")
        .expect("conversation");
    session
        .save(Save::new(&conversation.id).entry(
            "user",
            json!({"type": "image", "path": "/home/u/media/x.jpg", "mime": "image/jpeg"}),
        ))
        .expect("save image entry");

    let history = session.history(&conversation.id).expect("history");
    let entry = history.first().expect("the image entry");
    assert!(
        entry.tokens_est >= 3_000,
        "an image must be counted at its provider cost, got {}",
        entry.tokens_est
    );
}


#[test]
fn the_counts_are_read_back_from_a_saved_document() {
    use serde_json::json;

    let document = json!({"tokens": {
        "identity": 12, "tools": 34, "rules": 56, "skills": 78, "guidance": 90,
        "transcript": 111, "recall": 22, "runtime": 33, "total": 436, "fixed": 303
    }});
    let counts = TokenCounts::from_document(&document).expect("counts");
    assert_eq!(counts.identity, 12);
    assert_eq!(counts.guidance, 90);
    assert_eq!(counts.total, 436);
    assert_eq!(counts.fixed, 303);
    assert!(TokenCounts::from_document(&json!({"model": "m"})).is_none());
}
