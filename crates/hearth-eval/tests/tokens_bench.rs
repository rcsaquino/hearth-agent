#[test]
fn the_token_benchmark_runs_100_turns_and_reports_the_targets() {
    let report = hearth_eval::tokens_bench::run().expect("run the token benchmark");
    assert_eq!(report.turns, 100);
    assert!(report.compactions >= 1, "no compaction happened");
    assert!(
        report.prefix_stable,
        "the stable prefix changed during the run"
    );
    assert!(
        report.fixed_overhead_p95 <= 4_500,
        "fixed overhead p95 was {}",
        report.fixed_overhead_p95
    );
    assert!(
        report.cache_read_share >= 0.80,
        "cache-read share was {}",
        report.cache_read_share
    );
    assert!(
        report.assembly_ms_p95 <= 300.0,
        "assembly p95 was {} ms",
        report.assembly_ms_p95
    );
    assert!(
        report.recall_ms_p95 <= 80.0,
        "recall p95 was {} ms",
        report.recall_ms_p95
    );
    assert!(report.save_ms_p95 > 0.0);
    assert_eq!(report.per_turn.len(), 100);
    assert!(report.token_totals.contains_key("identity"));
    assert!(!report.targets.is_empty());
}
