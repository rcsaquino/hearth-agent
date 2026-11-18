use hearth_core::{EffectIntent, Harness, Save, SubmissionStatus, TelegramUpdate};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn conversations_are_found_by_transport_key() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    assert!(
        session
            .conversation_by_transport("telegram", "1:0")
            .expect("lookup")
            .is_none()
    );

    let conversation = session
        .create_conversation("Chat", "telegram", "1:0")
        .expect("create conversation");
    let found = session
        .conversation_by_transport("telegram", "1:0")
        .expect("lookup")
        .expect("conversation exists");
    assert_eq!(found.id, conversation.id);
    assert_eq!(found.transport, "telegram");
    assert_eq!(found.transport_key, "1:0");
    assert!(
        session
            .conversation_by_transport("telegram", "2:0")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn the_telegram_offset_defaults_to_zero_and_upserts() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    assert_eq!(session.telegram_offset().expect("offset"), 0);
    session.set_telegram_offset(5).expect("set offset");
    assert_eq!(session.telegram_offset().expect("offset"), 5);
    session.set_telegram_offset(6).expect("set offset");
    assert_eq!(session.telegram_offset().expect("offset"), 6);
}

#[test]
fn a_telegram_submission_saves_the_update_entry_submission_and_offset() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Chat", "telegram", "1:0")
        .expect("create conversation");

    let admission = session
        .submit_telegram(
            &conversation.id,
            "tg:1:10",
            json!({"text": "hello"}),
            TelegramUpdate {
                update_id: 42,
                chat_id: 1,
                message_id: 10,
            },
            43,
        )
        .expect("submit telegram");

    assert!(!admission.duplicate);
    assert_eq!(admission.submission.request_id, "tg:1:10");
    assert_eq!(admission.submission.status, SubmissionStatus::Queued);

    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, "user");
    assert_eq!(history[0].source.as_deref(), Some("10"));
    assert_eq!(history[0].content["text"], "hello");
    assert_eq!(session.telegram_offset().expect("offset"), 43);
    assert!(session.telegram_update_seen(42).expect("seen"));
    assert!(!session.telegram_update_seen(41).expect("seen"));
}

#[test]
fn a_duplicate_telegram_update_returns_the_existing_submission() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Chat", "telegram", "1:0")
        .expect("create conversation");
    let update = TelegramUpdate {
        update_id: 42,
        chat_id: 1,
        message_id: 10,
    };

    let first = session
        .submit_telegram(
            &conversation.id,
            "tg:1:10",
            json!({"text": "hello"}),
            update,
            43,
        )
        .expect("first admission");
    assert!(!first.duplicate);

    let second = session
        .submit_telegram(
            &conversation.id,
            "tg:1:10",
            json!({"text": "hello"}),
            update,
            43,
        )
        .expect("duplicate admission");
    assert!(second.duplicate);
    assert_eq!(second.submission.id, first.submission.id);
    assert_eq!(session.history(&conversation.id).expect("history").len(), 1);
    assert_eq!(session.telegram_offset().expect("offset"), 43);
}

#[test]
fn a_duplicate_update_with_a_new_request_id_is_rejected() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Chat", "telegram", "1:0")
        .expect("create conversation");
    let update = TelegramUpdate {
        update_id: 42,
        chat_id: 1,
        message_id: 10,
    };
    session
        .submit_telegram(
            &conversation.id,
            "tg:1:10",
            json!({"text": "hello"}),
            update,
            43,
        )
        .expect("first admission");

    let error = session
        .submit_telegram(
            &conversation.id,
            "tg:1:11",
            json!({"text": "different"}),
            update,
            44,
        )
        .expect_err("the update id is already used");
    assert!(matches!(error, hearth_core::CoreError::Storage(_)));
    assert_eq!(session.history(&conversation.id).expect("history").len(), 1);
    assert_eq!(session.telegram_offset().expect("offset"), 43);
}

#[test]
fn a_failed_telegram_save_leaves_no_entry_and_no_offset() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    let error = session
        .submit_telegram(
            "missing",
            "tg:1:10",
            json!({"text": "hello"}),
            TelegramUpdate {
                update_id: 42,
                chat_id: 1,
                message_id: 10,
            },
            43,
        )
        .expect_err("the conversation does not exist");
    assert!(matches!(
        error,
        hearth_core::CoreError::ConversationNotFound(_)
    ));
    assert_eq!(session.telegram_offset().expect("offset"), 0);
    assert!(!session.telegram_update_seen(42).expect("seen"));
}

#[test]
fn the_next_queued_telegram_submission_skips_local_submissions() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    let local = session
        .create_conversation("Local", "local", "local:1")
        .expect("create local conversation");
    session
        .submit(&local.id, "local:1", "input", json!({"text": "local"}))
        .expect("submit local");

    let telegram = session
        .create_conversation("Telegram", "telegram", "1:0")
        .expect("create telegram conversation");
    let admission = session
        .submit_telegram(
            &telegram.id,
            "tg:1:10",
            json!({"text": "hello"}),
            TelegramUpdate {
                update_id: 42,
                chat_id: 1,
                message_id: 10,
            },
            43,
        )
        .expect("submit telegram");

    let next = session
        .next_queued_telegram_submission()
        .expect("next queued")
        .expect("a telegram submission is queued");
    assert_eq!(next.id, admission.submission.id);

    session
        .settle(&admission.submission.id, SubmissionStatus::Done, None)
        .expect("settle");
    assert!(
        session
            .next_queued_telegram_submission()
            .expect("next queued")
            .is_none(),
        "a settled telegram submission must not be returned"
    );
}

#[test]
fn a_control_update_is_recorded_once_without_a_submission() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let update = TelegramUpdate {
        update_id: 50,
        chat_id: 1,
        message_id: 20,
    };

    assert!(
        session
            .record_telegram_update(update, 51)
            .expect("record update"),
        "the first insert should be new"
    );
    assert_eq!(session.telegram_offset().expect("offset"), 51);
    assert!(session.telegram_update_seen(50).expect("seen"));

    assert!(
        !session
            .record_telegram_update(update, 52)
            .expect("duplicate record"),
        "the second insert should be a duplicate"
    );
    assert_eq!(
        session.telegram_offset().expect("offset"),
        51,
        "a duplicate update must not advance the stored offset"
    );
}

#[test]
fn queued_submissions_are_counted_per_conversation() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let telegram = session
        .create_conversation("Telegram", "telegram", "1:0")
        .expect("create telegram");
    let other = session
        .create_conversation("Other", "telegram", "2:0")
        .expect("create other");

    assert_eq!(
        session
            .queued_submission_count(&telegram.id)
            .expect("count empty"),
        0
    );

    let first = session
        .submit_telegram(
            &telegram.id,
            "tg:1:10",
            json!({"text": "a"}),
            TelegramUpdate {
                update_id: 1,
                chat_id: 1,
                message_id: 10,
            },
            2,
        )
        .expect("submit first");
    session
        .submit_telegram(
            &telegram.id,
            "tg:1:11",
            json!({"text": "b"}),
            TelegramUpdate {
                update_id: 2,
                chat_id: 1,
                message_id: 11,
            },
            3,
        )
        .expect("submit second");
    session
        .submit(&other.id, "other:1", "input", json!({"text": "c"}))
        .expect("submit other");

    assert_eq!(
        session
            .queued_submission_count(&telegram.id)
            .expect("count telegram"),
        2
    );
    assert_eq!(
        session
            .queued_submission_count(&other.id)
            .expect("count other"),
        1
    );

    session
        .settle(&first.submission.id, SubmissionStatus::Done, None)
        .expect("settle first");
    assert_eq!(
        session
            .queued_submission_count(&telegram.id)
            .expect("count after settle"),
        1
    );
}

fn two_messages_share_an_answer(session: &hearth_core::Session) -> (String, String, String) {
    let conversation = session
        .create_conversation("Chat", "telegram", "1:0")
        .expect("create conversation");
    let active = session
        .submit_telegram(
            &conversation.id,
            "tg:1:10",
            json!({"text": "the first message"}),
            TelegramUpdate {
                update_id: 1,
                chat_id: 1,
                message_id: 10,
            },
            2,
        )
        .expect("submit active");
    let absorbed = session
        .submit_telegram(
            &conversation.id,
            "tg:1:11",
            json!({"text": "the second message"}),
            TelegramUpdate {
                update_id: 2,
                chat_id: 1,
                message_id: 11,
            },
            3,
        )
        .expect("submit absorbed");

    let answer = session
        .save(Save::new(&conversation.id).entry("assistant", json!({"text": "one answer"})))
        .expect("save answer");
    let answer_id = answer.entry_ids[0].clone();
    session
        .settle(
            &active.submission.id,
            SubmissionStatus::Done,
            Some(&answer_id),
        )
        .expect("settle active");
    session
        .settle(
            &absorbed.submission.id,
            SubmissionStatus::Done,
            Some(&answer_id),
        )
        .expect("settle absorbed");

    (
        conversation.id,
        active.submission.id,
        absorbed.submission.id,
    )
}

fn record_delivery(session: &hearth_core::Session, conversation: &str, submission: &str) {
    session
        .save(Save::new(conversation).effect(EffectIntent::new(
            "telegram.send",
            format!("tg:{submission}"),
            json!({"chat_id": 1, "markdown": "one answer"}),
        )))
        .expect("record the delivery");
}

#[test]
fn an_answer_shared_by_an_absorbed_submission_is_delivered_once() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let (conversation, active, _absorbed) = two_messages_share_an_answer(session);
    record_delivery(session, &conversation, &active);

    let owed = session
        .done_answers_without_delivery(0)
        .expect("read undelivered answers");
    assert!(
        owed.is_empty(),
        "the shared answer was delivered once and is not owed again: {owed:?}"
    );
}

#[test]
fn a_delivery_recorded_for_the_absorbed_submission_settles_the_shared_answer() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let (conversation, _active, absorbed) = two_messages_share_an_answer(session);
    record_delivery(session, &conversation, &absorbed);

    let owed = session
        .done_answers_without_delivery(0)
        .expect("read undelivered answers");
    assert!(
        owed.is_empty(),
        "a delivery recorded for any submission of the answer settles it: {owed:?}"
    );
}

fn stage(session: &hearth_core::Session, update_id: i64, message_id: i64, ready_at: i64) -> bool {
    session
        .stage_telegram_album_member(
            1,
            "album-1",
            None,
            TelegramUpdate {
                update_id,
                chat_id: 1,
                message_id,
            },
            json!({"message_id": message_id, "text": format!("photo {message_id}")}),
            ready_at,
            update_id + 1,
        )
        .expect("stage album member")
}

#[test]
fn staging_an_album_member_holds_it_until_the_window() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    assert!(stage(session, 1, 10, 2_000_000));
    assert!(session.telegram_album_pending().expect("pending"));
    assert_eq!(session.telegram_offset().expect("offset"), 2);
    assert!(
        session
            .due_telegram_albums(1_999_999)
            .expect("due")
            .is_empty(),
        "the album is not due before its ready time"
    );

    let due = session.due_telegram_albums(2_000_000).expect("due");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].chat_id, 1);
    assert_eq!(due[0].group_id, "album-1");
    assert_eq!(due[0].members.len(), 1);
    assert_eq!(due[0].members[0].message_id, 10);
    assert_eq!(due[0].members[0].message["text"], "photo 10");
}

#[test]
fn a_later_album_member_extends_the_window() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    assert!(stage(session, 1, 10, 2_000_000));
    assert!(stage(session, 2, 11, 2_500_000));

    assert!(
        session
            .due_telegram_albums(2_000_000)
            .expect("due")
            .is_empty(),
        "the second member moved the group ready time"
    );
    let due = session.due_telegram_albums(2_500_000).expect("due");
    assert_eq!(due.len(), 1);
    let message_ids: Vec<i64> = due[0]
        .members
        .iter()
        .map(|member| member.message_id)
        .collect();
    assert_eq!(message_ids, vec![10, 11], "members arrive in message order");
}

#[test]
fn a_replayed_album_member_does_not_extend_the_window() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();

    assert!(stage(session, 1, 10, 2_000_000));
    assert!(
        !stage(session, 1, 10, 3_000_000),
        "the same update id is not staged twice"
    );

    let due = session.due_telegram_albums(2_000_000).expect("due");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].members.len(), 1);
}

#[test]
fn an_album_submission_clears_its_rows() {
    let home = TempDir::new().expect("temp home");
    let harness = Harness::open(home.path()).expect("open harness");
    let session = harness.session();
    let conversation = session
        .create_conversation("Chat", "telegram", "1:0")
        .expect("create conversation");
    assert!(stage(session, 1, 10, 2_000_000));
    assert!(stage(session, 2, 11, 2_000_000));

    let admission = session
        .submit_telegram_album(
            &conversation.id,
            "tg:1:10",
            json!({"text": "one album"}),
            "10",
            1,
            "album-1",
        )
        .expect("submit album");

    assert!(!admission.duplicate);
    assert_eq!(admission.submission.request_id, "tg:1:10");
    assert!(!session.telegram_album_pending().expect("pending"));
    assert!(
        session
            .due_telegram_albums(i64::MAX)
            .expect("due")
            .is_empty(),
        "a submitted album leaves no staged rows"
    );
    let history = session.history(&conversation.id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].source.as_deref(), Some("10"));
    assert_eq!(history[0].content["text"], "one album");
}
