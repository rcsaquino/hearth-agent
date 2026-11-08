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
fn memory_scale_queries_match_fixture_memories() {
    let report = hearth_eval::memory_bench::run_scale(2_000).expect("scale benchmark");
    assert_eq!(report.items, 2_000);
    assert_eq!(report.fixture_rows, 195);
    assert_eq!(report.noise_rows, 1_805);
    assert_eq!(report.queries, 20);
    assert_eq!(
        report.matched, 20,
        "a scale query did not match a fixture memory"
    );
    assert!(
        report.retrieval_ms_p95 <= 80.0,
        "2k retrieval p95 was {} ms; {report:#?}",
        report.retrieval_ms_p95
    );
    assert!(
        report.real_recall_ms_p95 <= 80.0,
        "2k real recall p95 was {} ms; {report:#?}",
        report.real_recall_ms_p95
    );
    println!("{report:#?}");
}

#[test]
#[ignore]
fn memory_scale_200k() {
    let report = hearth_eval::memory_bench::run_scale(200_000).expect("scale benchmark");
    assert_eq!(report.items, 200_000);
    assert_eq!(report.fixture_rows, 195);
    assert_eq!(report.noise_rows, 199_805);
    assert_eq!(report.queries, 20);
    assert_eq!(
        report.matched, 20,
        "a scale query did not match a fixture memory"
    );
    assert!(
        report.retrieval_ms_p95 <= 80.0,
        "200k retrieval p95 was {} ms; {report:#?}",
        report.retrieval_ms_p95
    );
    assert!(
        report.real_recall_ms_p95 <= 80.0,
        "200k real recall p95 was {} ms; {report:#?}",
        report.real_recall_ms_p95
    );
    println!("{report:#?}");
}

#[test]
fn benchmark_spot_check_labels_are_honest() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/memory_bench.json")).expect("fixture");
    let questions = fixture["questions"].as_array().expect("questions");
    let mut checked = 0usize;
    for question in questions.iter().take(20) {
        assert_eq!(
            question["relevant_keys"], question["expected_keys"],
            "{} labels unrelated memories as relevant",
            question["key"]
        );
        checked += 1;
    }
    assert_eq!(checked, 20);
}

#[test]
fn benchmark_precision_is_measured_against_the_relevant_set() {
    let report = hearth_eval::memory_bench::run().expect("benchmark");
    assert!(
        report.precision_at_5 >= 0.80,
        "precision@relevant is {}; {report:#?}",
        report.precision_at_5
    );
}
