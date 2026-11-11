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
fn segment_budgets_match_the_spec() {
    assert_eq!(hearth_core::SEGMENT_IDENTITY, 800);
    assert_eq!(hearth_core::SEGMENT_TOOLS, 1_200);
    assert_eq!(hearth_core::SEGMENT_STANDING, 1_200);
    assert_eq!(hearth_core::SEGMENT_SKILLS, 300);
    assert_eq!(hearth_core::SEGMENT_RECALL, 900);
    assert_eq!(hearth_core::SEGMENT_RUNTIME, 150);
    assert_eq!(hearth_core::FIXED_OVERHEAD_TARGET, 4_500);
}

#[test]
fn token_counts_sum_and_report_the_fixed_overhead() {
    let counts = TokenCounts::new(10, 30, 40, 0, 0, 50, 60, 70);
    assert_eq!(counts.total, 260);
    assert_eq!(counts.fixed, 150);
    assert!(counts.over_budget().is_empty());
}

#[test]
fn an_over_budget_segment_is_named() {
    let counts = TokenCounts::new(900, 30, 40, 0, 0, 50, 60, 70);
    assert_eq!(counts.over_budget(), vec!["identity"]);
}
