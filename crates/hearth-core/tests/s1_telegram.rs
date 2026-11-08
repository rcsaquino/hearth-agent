use hearth_core::{Harness, SubmissionStatus, TelegramUpdate};
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
