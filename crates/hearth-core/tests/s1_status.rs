use hearth_core::{Harness, NewTask, NewUsage, StepOutput};
use hearth_model::Usage;
use serde_json::json;
use tempfile::TempDir;

fn harness() -> (TempDir, Harness) {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    (home, harness)
}

#[test]
fn status_reads_count_tasks_and_sum_usage() {
    let (_home, harness) = harness();
    let session = harness.session();
    let conversation = session
        .create_conversation("Test", "local", "local:1")
        .expect("conversation");

    let running = session
        .create_task(NewTask::new("test", 1, "start", json!({})))
        .expect("running task");
    let claimed = session
        .claim_task("owner", 60_000_000)
        .expect("claim")
        .expect("claimable");
    assert_eq!(claimed.id, running.id);

    let sleeping = session
        .create_task(NewTask::new("test", 1, "start", json!({})))
        .expect("sleeping task");
    let claimed = session
        .claim_task("owner", 60_000_000)
        .expect("claim")
        .expect("claimable");
    assert_eq!(claimed.id, sleeping.id);
    session
        .finish_task(
            &sleeping.id,
            "owner",
            StepOutput::Sleep {
                until_micros: i64::MAX / 2,
                phase: "wait".to_owned(),
                checkpoint: json!({}),
            },
        )
        .expect("sleep task");

    let counts = session.task_counts().expect("task counts");
    assert_eq!(counts.running, 1);
    assert_eq!(counts.asleep, 1);

    session
        .record_usage(NewUsage {
            task_id: Some(running.id.clone()),
            conversation_id: Some(conversation.id.clone()),
            provider: "test".to_owned(),
            model: "test-model".to_owned(),
            kind: "chat".to_owned(),
            usage: Usage {
                input_tokens: 10,
                cache_read_tokens: 4,
                cache_write_tokens: 0,
                output_tokens: 5,
                reasoning_tokens: 0,
            },
            latency_ms: 10,
        })
        .expect("record usage");

    let totals = session.usage_totals_since(0).expect("totals");
    assert_eq!(totals.input_tokens, 10);
    assert_eq!(totals.output_tokens, 5);
    assert_eq!(totals.cache_read_tokens, 4);
    let future = session.usage_totals_since(i64::MAX).expect("future totals");
    assert_eq!(future.input_tokens, 0);
    assert_eq!(future.output_tokens, 0);

    assert_eq!(session.memory_item_count().expect("memory count"), 0);
}
