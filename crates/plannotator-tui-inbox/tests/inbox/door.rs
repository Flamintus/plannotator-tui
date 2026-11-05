//! Discovery, pairing and the calls of the person's API, against the stand-in Inbox.

use std::sync::atomic::{AtomicUsize, Ordering};

use plannotator_tui_inbox::{
    AttachmentVersion, Client, Device, Error, Inbox, NewComment, NewReply, NewSubThread, Paired, Passage,
    ReviewTarget, SendMode, Token, connect, discover, token_path,
};
use serde_json::{Map, Value, json};

use crate::Result;
use crate::support::{
    CONNECTION_TOKEN, FakeInbox, Reply, Request, SERVER_SESSION, health, scratch, write_registry,
};

const DEVICE_TOKEN: &str = "tok_device";

fn record(id: &str) -> Value {
    json!({
        "id": id, "project_id": "prj_1", "thread_id": "msg_a1", "message_id": "msg_a1", "attachment_id": "",
        "path": "msg_a1", "version": "current", "message": { "at": "2026-11-05T10:00:00.000Z" },
        "annotation": { "id": id, "type": "COMMENT", "originalText": "words", "text": "why?" },
        "created_at": "2026-11-05T10:01:00.000Z", "updated_at": "2026-11-05T10:01:00.000Z",
        "removed_at": null, "sent_reply_id": null,
    })
}

fn paired(token: &str) -> Paired {
    Paired {
        token: Token::new(token),
        device: Device {
            id: "dev_1".into(),
            name: "tui".into(),
            platform: "terminal".into(),
            other: Map::new(),
        },
    }
}

fn pair_answer(token: &str) -> Reply {
    Reply::Json(
        201,
        json!({ "token": token, "device": { "id": "dev_1", "name": "tui", "platform": "terminal", "created_at": "x" } }),
    )
}

/// What the door answers each route the client calls, as the window would.
fn door(request: &Request) -> Reply {
    let path = request.path.split('?').next().unwrap_or_default();
    let route = path.strip_prefix("/api/inbox/device/").unwrap_or(path);
    let row = json!({ "thread_id": "msg_a1", "subject": "s", "thread_name": null, "section": "waiting",
        "unread": true, "last_at": "x", "resolved_at": null, "project": { "id": "prj_1", "name": "repo" } });
    let thread = json!({ "thread_id": "msg_a1", "subject": "s", "thread_name": null, "resolved_at": null, "messages": [
        { "id": "msg_a1", "thread_id": "msg_a1", "reply_to": null, "author": { "kind": "agent", "host": "claude-code",
          "session": "ses_a", "name": null }, "body": "Hello", "created_at": "x", "subject": "s" } ] });
    let body = match route {
        "/api/inbox/health" | "health" => return health(SERVER_SESSION),
        "sessions" => {
            json!({ "serverSession": SERVER_SESSION, "home": "/h", "projects": [ { "project": { "id": "prj_1",
            "name": "repo", "root": "/repo" }, "sessions": [ { "session": "ses_a", "host": "claude-code", "busy": false,
            "last_thread": null, "started_at": "x", "last_seen_at": "x", "idle_since": null } ] } ] })
        }
        "threads" => json!({ "sections": [ { "id": "waiting", "label": "Waiting on you", "threads": [row] },
            { "id": "new", "label": "New", "threads": [] } ] }),
        "threads/msg_a1" => {
            json!({ "serverSession": SERVER_SESSION, "cursor": 9, "thread": thread, "row": null })
        }
        "threads/msg_a1/comments" => {
            serde_json::from_str(include_str!("../fixtures/thread-view.json")).unwrap_or_default()
        }
        "attachments/att_1/view" => {
            json!({ "serverSession": SERVER_SESSION, "attachment": {}, "version": "current", "text": "# A", "html": null })
        }
        "review-targets" => json!({ "thread_id": "msg_r1", "message_id": "msg_r2", "created": true }),
        "threads/msg_a1/comment-sends" => json!({ "sends": [] }),
        _ if request.method == "POST" => json!({ "annotation": record("c1") }),
        _ => {
            return Reply::Json(
                404,
                json!({ "error": "Not a phone route.", "code": "device_route_not_found" }),
            );
        }
    };
    Reply::Json(200, body)
}

/// One field of a JSON body, `null` when it is absent.
fn field(body: &Value, name: &str) -> Value {
    body.get(name).cloned().unwrap_or(Value::Null)
}

fn client_of(inbox: &FakeInbox) -> Client {
    Client::new(&inbox.data_dir, Token::new(DEVICE_TOKEN))
}

#[test]
fn discovery_reads_the_registry_and_checks_the_inboxs_health() -> Result {
    let inbox = FakeInbox::start("discover", door)?;
    let found = discover(&inbox.data_dir)?;
    assert_eq!(found.base_url, format!("http://127.0.0.1:{}", inbox.port));
    assert_eq!((found.server_session.as_str(), found.version.as_str()), (SERVER_SESSION, "0.28.49-test"));
    assert!(!format!("{found:?}").contains(CONNECTION_TOKEN), "the connection token is never printed");
    Ok(())
}

#[test]
fn an_inbox_is_running_only_when_its_port_answers_the_registrys_session() -> Result {
    let never = scratch("never")?;
    assert!(matches!(discover(&never), Err(Error::NotRunning(_))), "no registry");
    let other = FakeInbox::start("other", |_| health("ss_other"))?;
    assert!(matches!(discover(&other.data_dir), Err(Error::NotRunning(_))), "another Inbox on the port");
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = closed.local_addr()?.port();
    drop(closed);
    write_registry(&never, port, SERVER_SESSION)?;
    assert!(matches!(discover(&never), Err(Error::NotRunning(_))), "nothing listens");
    Ok(())
}

#[test]
fn local_pairing_presents_the_connection_token_and_keeps_the_device_token() -> Result {
    let inbox = FakeInbox::start("pair", |request| match request.path.as_str() {
        "/api/inbox/device/pair-local" => pair_answer(DEVICE_TOKEN),
        _ => door(request),
    })?;
    connect(&inbox.data_dir, "tui")?.health()?;
    let pairing = inbox.requests_to("/api/inbox/device/pair-local");
    let [pairing] = pairing.as_slice() else { return Err("one pairing".into()) };
    assert_eq!(pairing.header("authorization"), Some(format!("Bearer {CONNECTION_TOKEN}").as_str()));
    assert_eq!(
        (field(&pairing.json(), "platform").as_str(), field(&pairing.json(), "name").as_str()),
        (Some("terminal"), Some("tui"))
    );
    let kept = Paired::load(&token_path(&inbox.data_dir))?.ok_or("token kept")?;
    assert_eq!(kept.token, Token::new(DEVICE_TOKEN));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(token_path(&inbox.data_dir))?.permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only");
    }
    // The kept token serves the next connection: no second pairing.
    connect(&inbox.data_dir, "tui")?;
    assert_eq!(inbox.requests_to("/api/inbox/device/pair-local").len(), 1);
    let door_calls = inbox.requests_to("/api/inbox/device/health");
    assert!(
        door_calls
            .iter()
            .all(|r| r.header("authorization") == Some("Bearer tok_device") && r.header("origin").is_none())
    );
    Ok(())
}

#[test]
fn a_revoked_token_is_forgotten_and_replaced() -> Result {
    let inbox = FakeInbox::start("revoked", |request| match request.path.as_str() {
        "/api/inbox/device/pair-local" => pair_answer("tok_new"),
        "/api/inbox/device/health" if request.header("authorization") == Some("Bearer tok_old") => {
            Reply::Json(401, json!({ "error": "removed", "code": "device_revoked" }))
        }
        _ => door(request),
    })?;
    paired("tok_old").save(&token_path(&inbox.data_dir))?;
    connect(&inbox.data_dir, "tui")?;
    assert_eq!(Paired::load(&token_path(&inbox.data_dir))?.map(|p| p.token), Some(Token::new("tok_new")));
    Ok(())
}

#[test]
fn every_call_takes_its_contract_route() -> Result {
    let inbox = FakeInbox::start("routes", door)?;
    let client = client_of(&inbox);
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
    client.send("msg_a1", SendMode::Now, &["local_3".to_owned()])?;
    client.send("msg_a1", SendMode::Round, &[])?;
    let called: Vec<String> = inbox.requests().iter().map(|r| format!("{} {}", r.method, r.path)).collect();
    let door = |rest: &str| rest.replacen(' ', " /api/inbox/device/", 1);
    let expected: Vec<String> = [
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
    ]
    .iter()
    .map(|route| door(route))
    .collect();
    assert_eq!(called, expected);
    let posts: Vec<Value> =
        inbox.requests().iter().filter(|r| r.method == "POST").map(Request::json).collect();
    let keys: std::collections::HashSet<&str> =
        posts.iter().filter_map(|b| b.get("idempotency_key").and_then(Value::as_str)).collect();
    assert_eq!(keys.len(), posts.len(), "every write has a key of its own");
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
        (field(&body(2), "attachment_id").as_str(), field(&body(2), "version").as_str()),
        (Some("att_1"), Some("current"))
    );
    assert_eq!(field(&body(3), "annotation"), json!({ "id": "local_3", "text": "and?" }));
    // A sub-thread names its passage; the server places it.
    assert_eq!(
        field(&body(4), "annotation"),
        json!({ "id": "local_4", "text": "here", "quote": "this part", "occurrence": 1 })
    );
    assert_eq!(field(&body(5), "resolved"), true);
    assert_eq!(
        (field(&body(7), "mode").as_str(), field(&body(7), "comment_ids")),
        (Some("now"), json!(["local_3"]))
    );
    assert_eq!((field(&body(8), "mode").as_str(), body(8).get("comment_ids")), (Some("round"), None));
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
    let sent = inbox.requests_to("/api/inbox/device/annotations/c1/resolve");
    let keys: Vec<Value> = sent.iter().map(|r| field(&r.json(), "idempotency_key")).collect();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys.first(), keys.get(1), "the door replays the first answer, if it landed");
    Ok(())
}

#[test]
fn a_refusal_is_read_from_the_inboxs_envelope() -> Result {
    let inbox = FakeInbox::start("refused", |request| match request.path.as_str() {
        "/api/inbox/device/annotations/c1/replies" => Reply::Json(
            422,
            json!({ "error": "annotation.text: the reply (1 to 10000 characters).", "code": "validation_error", "field": "annotation.text" }),
        ),
        "/api/inbox/device/annotations/c1/resolve" => Reply::Json(
            409,
            json!({ "error": "Sent comments are not kept in view.", "code": "persistent_comments_off" }),
        ),
        _ => door(request),
    })?;
    let client = client_of(&inbox);
    let refused = client.reply("c1", &NewReply { id: "local_5".into(), text: String::new() });
    assert!(
        matches!(refused, Err(Error::Refused { status: 422, field: Some(ref f), .. }) if f == "annotation.text")
    );
    assert!(matches!(client.resolve("c1", true), Err(Error::SettingOff { .. })));
    // A route the door does not carry yet (an Inbox before E7).
    assert!(matches!(client.thread_view("msg_zz"), Err(Error::RouteMissing { .. })));
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
    assert_eq!(
        (
            first.requests_to("/api/inbox/device/health").len(),
            second.requests_to("/api/inbox/device/health").len()
        ),
        (1, 1)
    );
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
