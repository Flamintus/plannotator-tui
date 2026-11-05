//! The Inbox review against a stand-in Inbox (`crate::inbox::stand_in`): opening its
//! documents (`open`), the comments it keeps (`kept`), sending them (`send`), following the
//! thread live (`live`).

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

mod kept;
mod live;
mod open;
mod send;

use std::rc::Rc;

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::json;

use super::super::App;
use crate::delivery::Discard;
use crate::inbox::import::Depositor;
use crate::inbox::source::thread_documents;
use crate::inbox::stand_in::{StandIn, attached, thread};

/// An Inbox with one thread: two agent messages, the first with a Markdown file and a
/// picture attached, and the person's message between them.
pub(super) fn thread_inbox() -> Rc<StandIn> {
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
    inbox
}

/// A review of `inbox`'s thread `thr_1`, the picker up on its documents.
pub(super) fn thread_app_of(inbox: &Rc<StandIn>) -> App {
    let found = thread_documents(inbox.as_ref(), "thr_1", false).expect("documents");
    App::open_inbox_thread(found, inbox.clone(), 80, Box::new(Discard)).expect("app")
}

pub(super) fn thread_app() -> (App, Rc<StandIn>) {
    let inbox = thread_inbox();
    (thread_app_of(&inbox), inbox)
}

pub(super) fn press(app: &mut App, code: KeyCode) {
    app.handle_event(&Event::Key(KeyEvent::from(code))).expect("key");
}

/// `last` on two replies, newest first, of the Inbox's live session `ses_a`.
pub(super) fn last_app(newest: bool) -> (App, Rc<StandIn>) {
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
