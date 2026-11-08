use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use hearth_core::{ClaimedTask, Harness, NewTask, Save, Session, StepOutput, SubmissionStatus};
use hearth_memory::MemoryService;
use hearth_model::{
    ChatRequest, Pricing, Provider, ProviderError, ProviderStream, StreamEvent, Usage,
};
use hearth_runtime::TaskHandler;
use hearth_runtime::skill_review::{ReviewReport, SkillReviewHandler, SkillReviewer};
use hearth_runtime::skills::{SkillLibrary, SkillSave, SkillStatus};
use serde_json::json;
use tempfile::TempDir;

struct Scripted {
    responses: Mutex<VecDeque<String>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl Scripted {
    fn new(response: &str) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::from([response.to_owned()])),
            requests: Mutex::new(Vec::new()),
        })
    }
}

impl Provider for Scripted {
    fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().expect("requests").push(request);
        let text = self.responses.lock().expect("responses").pop_front().unwrap_or_default();
        let events: Vec<Result<StreamEvent, ProviderError>> = vec![
            Ok(StreamEvent::TextDelta(text)),
            Ok(StreamEvent::Usage(Usage {
                input_tokens: 20,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 7,
                reasoning_tokens: 0,
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".to_owned()),
            }),
        ];
        Ok(Box::new(events.into_iter()))
    }
}

fn body() -> &'static str {
    "## When to use\n\nWhen a request needs triage.\n\n## Steps\n\nRead it, sort it, answer it.\n\n## Pitfalls\n\nDo not overcomplicate it."
}

fn reviewer(session: &Session, home: &Path, response: &str) -> (Arc<Scripted>, SkillReviewer) {
    let scripted = Scripted::new(response);
    let memory = Arc::new(MemoryService::new(session.clone()));
    let library = Arc::new(SkillLibrary::new(home));
    let reviewer = SkillReviewer::new(
        session.clone(),
        scripted.clone(),
        Pricing::default(),
        memory,
        library,
        "model".to_owned(),
        "openai-compatible".to_owned(),
        1024,
    );
    (scripted, reviewer)
}

fn seed_run(session: &Session, conversation: &str, skill: &str, request: &str) -> String {
    let submitted = session
        .submit(
            conversation,
            request,
            "input",
            json!({"text": "use the workflow"}),
        )
        .expect("submit");
    let user = session
        .latest_user_entry_id(conversation)
        .expect("user lookup")
        .expect("user entry");
    let assistant = session
        .save(Save::new(conversation).entry(
            "assistant",
            json!([{
                "type": "tool_call",
                "id": format!("call-{request}"),
                "name": "skill",
                "arguments": {"action": "load", "name": skill},
            }]),
        ))
        .expect("assistant entry");
    let answer = assistant.entry_ids[0].clone();
    session
        .settle(&submitted.submission.id, SubmissionStatus::Done, Some(&answer))
        .expect("settle");
    user
}

fn run_review(reviewer: &SkillReviewer, session: &Session) -> (ReviewReport, String) {
    let task = session
        .create_task(NewTask::new("skill.review", 1, "run", json!({})))
        .expect("review task");
    let report = reviewer.run(&task.id, session.now_micros()).expect("review");
    (report, task.id)
}

fn learned_file(
    home: &Path,
    name: &str,
    status: &str,
    uses: u64,
    successes: u64,
    failures: u64,
    last_used: &str,
) {
    let path = home.join("skills/learned").join(format!("{name}.md"));
    fs::create_dir_all(path.parent().expect("parent")).expect("learned dir");
    fs::write(
        &path,
        format!(
            "---\nname: {name}\ndescription: {name} skill\nversion: 1\nstatus: {status}\ncreated: 2020-01-01\nupdated: 2020-01-01\nlast_used: {last_used}\nuses: {uses}\nsuccesses: {successes}\nfailures: {failures}\nevidence: [entry-1]\n---\n\n{}\n",
            body()
        ),
    )
    .expect("write learned skill");
}

fn conversation(session: &Session) -> String {
    session
        .create_conversation("Review", "local", "local:review")
        .expect("conversation")
        .id
}

#[test]
fn a_review_creates_a_skill_with_three_evidence_runs() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let conversation = conversation(session);
    let library = SkillLibrary::new(home.path());
    library
        .save_learned(SkillSave {
            name: "research".to_owned(),
            description: "Research a topic".to_owned(),
            body: body().to_owned(),
            evidence: vec!["entry-1".to_owned()],
        })
        .expect("learned skill");
    let evidence: Vec<String> = (0..3)
        .map(|index| seed_run(session, &conversation, "research", &format!("req-{index}")))
        .collect();
    let response = json!({
        "changes": [{
            "action": "create",
            "name": "triage",
            "description": "Triage a request",
            "body": body(),
            "evidence": evidence,
        }]
    })
    .to_string();
    let (scripted, reviewer) = reviewer(session, home.path(), &response);
    let (report, task_id) = run_review(&reviewer, session);
    assert!(report.applied >= 1, "{report:?}");
    let stored = SkillLibrary::new(home.path());
    let triage = stored.find("triage").expect("find").expect("triage");
    assert_eq!(triage.status, SkillStatus::Draft);
    let research = stored.find("research").expect("find").expect("research");
    assert_eq!(research.status, SkillStatus::Active);
    let usage = session.usage_for_task(&task_id).expect("usage");
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].kind, "skill.review");
    let prompt = session.latest_prompt().expect("prompt").expect("blob");
    assert!(prompt.text.contains("Recent successful runs"), "{}", prompt.text);
    assert_eq!(scripted.requests.lock().expect("requests").len(), 1);
}

#[test]
fn a_review_rejects_a_create_with_too_little_evidence() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let conversation = conversation(session);
    let evidence = vec![seed_run(session, &conversation, "research", "req-1")];
    let response = json!({
        "changes": [{
            "action": "create",
            "name": "triage",
            "description": "Triage a request",
            "body": body(),
            "evidence": evidence,
        }]
    })
    .to_string();
    let (_scripted, reviewer) = reviewer(session, home.path(), &response);
    let (report, _) = run_review(&reviewer, session);
    assert_eq!(report.rejected, 1);
    assert!(SkillLibrary::new(home.path()).find("triage").expect("find").is_none());
}

#[test]
fn a_review_cannot_edit_a_user_skill() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let conversation = conversation(session);
    let path = home.path().join("skills/writing.md");
    fs::create_dir_all(path.parent().expect("parent")).expect("skills dir");
    fs::write(
        &path,
        format!("---\nname: writing\ndescription: Write clearly\n---\n\n{}\n", body()),
    )
    .expect("user skill");
    let evidence: Vec<String> = (0..3)
        .map(|index| seed_run(session, &conversation, "writing", &format!("req-{index}")))
        .collect();
    let response = json!({
        "changes": [{
            "action": "revise",
            "name": "writing",
            "description": "Write badly",
            "body": body(),
            "evidence": evidence,
        }]
    })
    .to_string();
    let (_scripted, reviewer) = reviewer(session, home.path(), &response);
    let (report, _) = run_review(&reviewer, session);
    assert_eq!(report.rejected, 1);
    let written = fs::read_to_string(&path).expect("read user skill");
    assert!(written.contains("Write clearly"), "{written}");
}

#[test]
fn a_review_caps_changes_at_three() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let conversation = conversation(session);
    let evidence: Vec<String> = (0..3)
        .map(|index| seed_run(session, &conversation, "research", &format!("req-{index}")))
        .collect();
    let changes: Vec<serde_json::Value> = (0..4)
        .map(|index| {
            json!({
                "action": "create",
                "name": format!("cap-{index}"),
                "description": format!("Cap skill {index}"),
                "body": body(),
                "evidence": evidence,
            })
        })
        .collect();
    let response = json!({"changes": changes}).to_string();
    let (_scripted, reviewer) = reviewer(session, home.path(), &response);
    let (report, _) = run_review(&reviewer, session);
    assert_eq!(report.applied, 3);
    let stored = SkillLibrary::new(home.path());
    assert!(stored.find("cap-0").expect("find").is_some());
    assert!(stored.find("cap-1").expect("find").is_some());
    assert!(stored.find("cap-2").expect("find").is_some());
    assert!(stored.find("cap-3").expect("find").is_none());
}

#[test]
fn the_automatic_lifecycle_promotes_quarantines_and_archives() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    learned_file(home.path(), "research", "draft", 1, 1, 0, "2026-11-01");
    learned_file(home.path(), "danger", "active", 2, 0, 2, "2026-11-01");
    learned_file(home.path(), "old", "active", 1, 1, 0, "2020-01-01T00:00:00Z");
    let response = json!({"changes": []}).to_string();
    let (_scripted, reviewer) = reviewer(session, home.path(), &response);
    let (report, _) = run_review(&reviewer, session);
    assert_eq!(report.applied, 3);
    let stored = SkillLibrary::new(home.path());
    assert_eq!(
        stored.find("research").expect("find").expect("research").status,
        SkillStatus::Active
    );
    assert_eq!(
        stored.find("danger").expect("find").expect("danger").status,
        SkillStatus::Quarantined
    );
    assert_eq!(
        stored.find("old").expect("find").expect("old").status,
        SkillStatus::Archived
    );
}

#[test]
fn the_review_handler_records_the_night_and_sleeps() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("harness");
    let session = harness.session();
    let (_scripted, reviewer) = reviewer(session, home.path(), r#"{"changes":[]}"#);
    let handler = SkillReviewHandler::new(session.clone(), Arc::new(reviewer));
    let task = ClaimedTask {
        id: "task-skill-review".to_owned(),
        conversation_id: None,
        kind: "skill.review".to_owned(),
        phase: "run".to_owned(),
        attempt: 1,
        input: json!({"slot": "04:30"}),
        checkpoint: None,
    };
    let output = handler.step(&task).expect("review handler");
    let StepOutput::Sleep { until_micros, .. } = output else {
        panic!("expected sleep");
    };
    assert!(until_micros > session.now_micros());
    assert!(session
        .meta_value("last_skill_review")
        .expect("meta")
        .is_some());
    assert_eq!(
        session.meta_value("last_skill_review_changes").expect("meta"),
        Some("0".to_owned())
    );
}
