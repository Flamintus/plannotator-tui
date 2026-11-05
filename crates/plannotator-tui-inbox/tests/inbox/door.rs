//! The calls of the person's API, against the stand-in Inbox.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use plannotator_tui_inbox::{
    AttachmentVersion, Client, Error, IdempotencyKey, Inbox, NewComment, NewReply, NewSubThread, Passage,
    ReviewTarget, SendMode,
};
use serde_json::{Value, json};

use crate::Result;
use crate::support::{DOOR, FakeInbox, Reply, client_of, door, door_requests, field, posts};

/// Every call of the API once, in a fixed order.
fn call_everything(client: &Client, press: &IdempotencyKey) -> Result {
    client.health()?;
    assert_eq!(client.sessions()?.first().map(|p| p.sessions.len()), Some(1));
    assert_eq!(client.threads()?.len(), 1, "every section's rows");
    assert_eq!(client.thread("msg_a1")?.messages.first().map(|m| m.body.as_str()), Some("Hello"));
    assert_eq!(client.thread_view("msg_a1")?.v, 1);
    assert_eq!(client.attachment_text("att_1", AttachmentVersion::Sent)?.text, "# A");
    client.deposit(&ReviewTarget::chat_message("claude-code", "ses_a", "msg_01", "The reply."))?;
    client.comment_on_message("msg_a1", &NewComment::new("local_1", "words", "why?"))?;
    client.comment_on_attachment("att_1", "current", &NewComment::new("local_2", "line", "rename"))?;
    client.reply("c1", &NewReply { id: "local_3".into(), text: "and?".into() })?;
    let on = Passage { parent: "rep_a".into(), quote: "this part".into(), occurrence: 1 };
    client.sub_thread(&NewSubThread { id: "local_4".into(), on, text: "here".into() })?;
    client.resolve("c1", true)?;
    client.remove("local_1")?;
    client.send("msg_a1", SendMode::Now, &["local_3".to_owned()], press)?;
    client.send("msg_a1", SendMode::Round, &[], &IdempotencyKey::new())?;
    Ok(())
}

#[test]
fn every_call_takes_its_contract_route() -> Result {
    let inbox = FakeInbox::start("routes", door)?;
    call_everything(&client_of(&inbox), &IdempotencyKey::new())?;
    let called: Vec<String> = door_requests(&inbox)
        .iter()
        .map(|r| format!("{} {}", r.method, r.path.strip_prefix(DOOR).unwrap_or(&r.path)))
        .collect();
    let expected = [
        "GET health",
        "GET sessions",
        "GET threads",
        "GET threads/msg_a1",
        "GET threads/msg_a1/comments?v=1",
        "GET attachments/att_1/view?version=sent",
        "POST review-targets",
        "POST messages/msg_a1/annotations",
        "POST annotations",
        "POST annotations/c1/replies",
        "POST annotations/rep_a/subthreads",
        "POST annotations/c1/resolve",
        "POST annotations/local_1/remove",
        "POST threads/msg_a1/comment-sends",
        "POST threads/msg_a1/comment-sends",
    ];
    assert_eq!(called, expected);
    Ok(())
}

#[test]
fn every_write_sends_its_contract_body() -> Result {
    let inbox = FakeInbox::start("bodies", door)?;
    call_everything(&client_of(&inbox), &IdempotencyKey::new())?;
    let posts = posts(&inbox);
    let body = |n: usize| posts.get(n).cloned().unwrap_or_default();
    assert_eq!(field(&body(0), "session"), "claude-code:ses_a");
    assert_eq!(
        field(&body(0), "target"),
        json!({ "kind": "chat_message", "text": "The reply.", "host_message_id": "msg_01" })
    );
    assert_eq!(
        field(&body(1), "annotation"),
        json!({ "id": "local_1", "type": "COMMENT", "originalText": "words", "text": "why?" })
    );
    assert_eq!(
        (field(&body(2), "attachment_id"), field(&body(2), "version")),
        (json!("att_1"), json!("current"))
    );
    assert_eq!(field(&body(3), "annotation"), json!({ "id": "local_3", "text": "and?" }));
    // A sub-thread names its passage; the server places it.
    assert_eq!(
        field(&body(4), "annotation"),
        json!({ "id": "local_4", "text": "here", "quote": "this part", "occurrence": 1 })
    );
    assert_eq!(field(&body(5), "resolved"), true);
    assert_eq!((field(&body(7), "mode"), field(&body(7), "comment_ids")), (json!("now"), json!(["local_3"])));
    assert_eq!((field(&body(8), "mode"), body(8).get("comment_ids").cloned()), (json!("round"), None));
    Ok(())
}

#[test]
fn every_write_goes_under_a_key_of_its_own_and_a_send_under_its_press() -> Result {
    let inbox = FakeInbox::start("keys", door)?;
    let press = IdempotencyKey::new();
    call_everything(&client_of(&inbox), &press)?;
    let keys: Vec<Value> = posts(&inbox).iter().map(|b| field(b, "idempotency_key")).collect();
    let distinct: HashSet<String> = keys.iter().filter_map(Value::as_str).map(str::to_owned).collect();
    assert_eq!(distinct.len(), keys.len());
    assert_eq!(keys.get(7), Some(&json!(press.as_str())));
    Ok(())
}

#[test]
fn the_door_gets_the_device_token_and_never_an_origin() -> Result {
    let inbox = FakeInbox::start("bearer", door)?;
    call_everything(&client_of(&inbox), &IdempotencyKey::new())?;
    let sent = door_requests(&inbox);
    assert!(sent.iter().all(|r| r.header("authorization") == Some("Bearer tok_device")));
    assert!(sent.iter().all(|r| r.header("origin").is_none()));
    Ok(())
}

#[test]
fn a_write_that_did_not_complete_is_sent_again_under_the_same_key() -> Result {
    let tries = AtomicUsize::new(0);
    let inbox = FakeInbox::start("retry", move |request| {
        if request.path.ends_with("/resolve") && tries.fetch_add(1, Ordering::SeqCst) == 0 {
            return Reply::Hang;
        }
        door(request)
    })?;
    client_of(&inbox).resolve("c1", false)?;
    let keys: Vec<Value> = posts(&inbox).iter().map(|b| field(b, "idempotency_key")).collect();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys.first(), keys.get(1), "the door replays the first answer, if it landed");
    Ok(())
}

#[test]
fn a_refused_write_is_not_sent_again() -> Result {
    let inbox = FakeInbox::start("no-retry", |request| match request.path.as_str() {
        "/api/inbox/device/annotations/c1/replies" => Reply::Json(
            422,
            json!({ "error": "annotation.text: the reply (1 to 10000 characters).", "code": "validation_error", "field": "annotation.text" }),
        ),
        _ => door(request),
    })?;
    let refused = client_of(&inbox).reply("c1", &NewReply { id: "local_5".into(), text: String::new() });
    assert!(
        matches!(refused, Err(Error::Refused { status: 422, field: Some(ref f), .. }) if f == "annotation.text")
    );
    assert_eq!(posts(&inbox).len(), 1);
    Ok(())
}

#[test]
fn a_refusal_is_read_from_the_inboxs_envelope() -> Result {
    let inbox = FakeInbox::start("refused", |request| match request.path.as_str() {
        "/api/inbox/device/annotations/c1/resolve" => Reply::Json(
            409,
            json!({ "error": "Sent comments are not kept in view.", "code": "persistent_comments_off" }),
        ),
        _ => door(request),
    })?;
    let client = client_of(&inbox);
    assert!(matches!(client.resolve("c1", true), Err(Error::SettingOff { .. })));
    // A route the door does not carry yet (an Inbox before E7).
    assert!(matches!(client.thread_view("msg_zz"), Err(Error::RouteMissing { .. })));
    Ok(())
}

#[test]
fn a_now_send_names_its_drafts() -> Result {
    let inbox = FakeInbox::start("now-empty", door)?;
    let sent = client_of(&inbox).send("msg_a1", SendMode::Now, &[], &IdempotencyKey::new());
    assert!(matches!(sent, Err(Error::Invalid(_))));
    assert_eq!(posts(&inbox), Vec::<Value>::new());
    Ok(())
}

#[test]
fn the_registry_is_read_again_for_every_call() -> Result {
    let first = FakeInbox::start("moved", door)?;
    let client = client_of(&first);
    client.health()?;
    // The Inbox restarted on another port: the next call goes there.
    let second = FakeInbox::serve(first.data_dir.clone(), door)?;
    client.health()?;
    let health = "/api/inbox/device/health";
    assert_eq!((first.requests_to(health).len(), second.requests_to(health).len()), (1, 1));
    Ok(())
}

#[test]
fn a_view_in_a_newer_contract_is_refused() -> Result {
    let inbox = FakeInbox::start("newer", |request| match request.path.as_str() {
        "/api/inbox/device/threads/msg_a1/comments?v=1" => {
            Reply::Json(200, json!({ "v": 2, "round": 1, "items": [] }))
        }
        _ => door(request),
    })?;
    assert!(matches!(client_of(&inbox).thread_view("msg_a1"), Err(Error::Newer { v: 2, .. })));
    Ok(())
}
