//! Scale measurements for the memory store.
//!
//! These are ignored by default because they seed large stores. They exist so a
//! reported number can be re-run instead of extrapolated, and so a bound on the
//! turn path has a test that would fail if the work grew back.

use std::time::Instant;

use hearth_core::Harness;
use hearth_memory::{MemoryService, NewMemory};
use tempfile::TempDir;

fn service() -> (TempDir, MemoryService) {
    let directory = TempDir::new().expect("temp home");
    let harness = Harness::open(directory.path()).expect("harness");
    let service = MemoryService::new(harness.session().clone());
    (directory, service)
}

fn seed(service: &MemoryService, count: usize) {
    let mut items = Vec::with_capacity(count);
    for index in 0..count {
        items.push(
            NewMemory::new(format!("topic {index} notes the decision for case {index}"))
                .subject(format!("topic {index}")),
        );
    }
    service.write_many(items).expect("seed");
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() as f64 - 1.0) * fraction).round() as usize;
    values[index]
}

#[test]
#[ignore = "seeds a large store"]
fn the_subject_scan_curve() {
    let sizes: Vec<usize> = std::env::var("HEARTH_SCALE_SIZES")
        .unwrap_or_else(|_| "1000,10000,100000".to_owned())
        .split(',')
        .filter_map(|value| value.trim().parse().ok())
        .collect();
    for size in sizes {
        let (_directory, service) = service();
        let seeded = Instant::now();
        seed(&service, size);
        let seed_ms = seeded.elapsed().as_secs_f64() * 1_000.0;
        let query = format!("what did we decide about topic {}?", size / 2);

        let subject_count: i64 = service
            .session()
            .storage()
            .read(|reader| {
                reader.query_row(
                    "SELECT COUNT(DISTINCT subject) FROM memories WHERE subject <> ''",
                    [],
                    |row| row.get(0),
                )
            })
            .expect("subject count");
        let mut subject_times = Vec::new();
        for _ in 0..10 {
            let started = Instant::now();
            let listed: Vec<String> = service
                .session()
                .storage()
                .read(|reader| {
                    reader.query(
                        "SELECT DISTINCT subject FROM memories WHERE subject <> '' \
                         ORDER BY LENGTH(subject) DESC, subject LIMIT 4096",
                        [],
                        |row| row.get(0),
                    )
                })
                .expect("subjects");
            subject_times.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(listed.len(), (subject_count as usize).min(4_096));
        }
        let subject_p95 = percentile(&mut subject_times, 0.95);

        // A fresh service has an empty subject cache, so each run pays the full
        // scan the first recall after a write would pay.
        let mut cold = Vec::new();
        for _ in 0..10 {
            let fresh = MemoryService::new(service.session().clone());
            let started = Instant::now();
            let recall = fresh.recall(&query).expect("recall");
            cold.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert!(!recall.digest.is_empty(), "the query recalled nothing");
        }

        let started = Instant::now();
        for _ in 0..50 {
            service.recall(&query).expect("recall");
        }
        let warm = started.elapsed().as_secs_f64() * 1_000.0 / 50.0;

        println!(
            "size {size}: subjects {subject_count} · seed {seed_ms:.0} ms · subject query p95 {subject_p95:.2} ms · cold p50 {:.2} ms · cold p95 {:.2} ms · warm {:.2} ms",
            percentile(&mut cold.clone(), 0.50),
            percentile(&mut cold, 0.95),
            warm
        );
        assert!(
            subject_p95 < 25.0,
            "the subject query took {subject_p95:.2} ms at p95 on {size} statements"
        );
    }
}
