#[test]
fn memory_benchmark_runs() {
    let report = hearth_eval::memory_bench::run().expect("benchmark");
    assert_eq!(report.questions, 200);
    assert_eq!(report.fixture_memories, 200);
    assert_eq!(report.spot_check, 20);
    assert_eq!(report.secret_leaks, 0);
    assert_eq!(report.duplicate_failures, 0);
    assert!(report.stored_rows <= report.fixture_memories);
    assert!(report.paraphrase.is_some());
    assert!(report.categories.contains_key("direct"));
    println!("{report:#?}");
}

#[test]
#[ignore]
fn memory_scale_200k() {
    let report = hearth_eval::memory_bench::run_scale(200_000).expect("scale benchmark");
    assert_eq!(report.items, 200_000);
    assert_eq!(report.queries, 20);
    println!("{report:#?}");
}
