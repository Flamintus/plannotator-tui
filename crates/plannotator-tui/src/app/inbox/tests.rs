#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_schema::Provenance;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::json;

use super::super::{App, Mode};
use crate::delivery::Discard;
use crate::inbox::import::Depositor;
use crate::inbox::source::thread_documents;
use crate::inbox::stand_in::{StandIn, attached, thread};

/// An Inbox thread: two agent messages, the first with a Markdown file and a picture
/// attached, and the person's message between them.
fn thread_app() -> (App, Rc<StandIn>) {
    let inbox = Rc::new(StandIn::default());
    {
        let mut state = inbox.state.borrow_mut();
        state.threads.push(thread(
            "thr_1",
            "The plan",
            &[
                (
                    "msg_a1",
                    true,
                    "First answer.",
                    json!([attached("att_1", "plan.md", "markdown"), attached("att_2", "shot.png", "image")]),
                ),
                ("msg_p1", false, "Go on.", json!([])),
                ("msg_a2", true, "Second answer.", json!([])),
            ],
        ));
        let words = serde_json::from_value(json!({ "version": "current", "text": "# Plan\n\nShip it.\n" }));
        state.files.push(("att_1".into(), words.expect("words")));
    }
    let found = thread_documents(inbox.as_ref(), "thr_1", false).expect("documents");
    let app = App::open_inbox_thread(found, 80, Box::new(Discard)).expect("app");
    (app, inbox)
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_event(&Event::Key(KeyEvent::from(code))).expect("key");
}

#[test]
fn an_inbox_thread_offers_its_agent_messages_then_their_text_files_newest_first() {
    let (app, _) = thread_app();
    let offered: Vec<&str> = app.candidates.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(offered, ["Second answer.", "First answer.", "file plan.md"]);
    assert_eq!(app.mode, Mode::Pick);
    assert_eq!(
        app.open.source.provenance,
        Provenance::Inbox { thread_id: "thr_1".into(), message_id: "msg_a2".into(), attachment_id: None }
    );
    assert_eq!(app.open.doc.source, "Second answer.");
}

#[test]
fn a_picked_file_opens_with_its_words_and_names_its_attachment() {
    let (mut app, _) = thread_app();
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.open.doc.source, "# Plan\n\nShip it.\n");
    assert_eq!(
        app.open.source.provenance,
        Provenance::Inbox {
            thread_id: "thr_1".into(),
            message_id: "msg_a1".into(),
            attachment_id: Some("att_1".into()),
        }
    );
    assert!(app.open.source.transient, "the Inbox keeps what is said about it, not a local record");
}

/// `last` on two replies, newest first, of the Inbox's live session `ses_a`.
fn last_app(newest: bool) -> (App, Rc<StandIn>) {
    let inbox = Rc::new(StandIn::with_session());
    let reply = |id: &str, text: &str| plannotator_tui_hosts::Message {
        id: id.into(),
        role: plannotator_tui_hosts::Role::Assistant,
        text: text.into(),
        at: None,
    };
    let replies = vec![reply("m2", "Second reply.\n"), reply("m1", "First reply.\n")];
    let mut app = App::open_message(
        "claude",
        "/tmp/transcript.jsonl",
        Some("ses_a"),
        replies,
        80,
        Box::new(Discard),
        newest,
    )
    .expect("app");
    let depositor = Depositor::find(inbox.clone(), "ses_a").expect("the session is live");
    app.review_in_inbox(Ok(Some(depositor)));
    (app, inbox)
}

fn deposited(inbox: &StandIn) -> Vec<String> {
    let state = inbox.state.borrow();
    state
        .deposits
        .iter()
        .map(|(target, _)| match &target.target {
            plannotator_tui_inbox::Target::ChatMessage { host_message_id, .. } => {
                host_message_id.clone().unwrap_or_default()
            }
            plannotator_tui_inbox::Target::File { path } => path.clone(),
        })
        .collect()
}

#[test]
fn the_reply_under_review_is_deposited_in_its_sessions_review_thread() {
    let (app, inbox) = last_app(true);
    assert_eq!(deposited(&inbox), ["m2"]);
    let state = inbox.state.borrow();
    let (target, _) = &state.deposits[0];
    assert_eq!(target.session, "claude-code:ses_a", "the Inbox's name of the host, and the host's id");
    assert_eq!(target.project_path.as_deref(), Some("/repo"));
    assert_eq!(
        app.open.source.provenance,
        Provenance::Inbox {
            thread_id: "thr_review".into(),
            message_id: "msg_r1".into(),
            attachment_id: None
        }
    );
    assert_eq!(app.open.doc.source, "Second reply.\n");
}

#[test]
fn a_reply_is_deposited_once_it_is_picked_never_while_previewed() {
    let (mut app, inbox) = last_app(false);
    assert_eq!(app.mode, Mode::Pick);
    press(&mut app, KeyCode::Down);
    assert!(deposited(&inbox).is_empty(), "a preview stays in the picker");
    press(&mut app, KeyCode::Enter);
    assert_eq!(deposited(&inbox), ["m1"]);
    assert!(matches!(app.open.source.provenance, Provenance::Inbox { .. }));
}

#[test]
fn a_reply_the_inbox_refuses_stays_with_the_agents_pane() {
    let inbox = Rc::new(StandIn::with_session());
    inbox.state.borrow_mut().missing.push("review-targets");
    let reply = plannotator_tui_hosts::Message {
        id: "m1".into(),
        role: plannotator_tui_hosts::Role::Assistant,
        text: "A reply.\n".into(),
        at: None,
    };
    let mut app =
        App::open_message("claude", "/t.jsonl", Some("ses_a"), vec![reply], 80, Box::new(Discard), true)
            .expect("app");
    app.review_in_inbox(Ok(Some(Depositor::find(inbox.clone(), "ses_a").expect("live"))));
    assert!(matches!(app.open.source.provenance, Provenance::AgentMessage { .. }));
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("feedback goes to the agent as before"), "{status}");
}

#[test]
fn a_session_the_inbox_does_not_know_is_never_reviewed_there() {
    let inbox = Rc::new(StandIn::with_session());
    let err = Depositor::find(inbox, "ses_other").expect_err("not a live session");
    assert!(format!("{err:#}").contains("no live session ses_other"), "{err:#}");
}
