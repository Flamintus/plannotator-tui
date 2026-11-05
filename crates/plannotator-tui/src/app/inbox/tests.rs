#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_schema::Provenance;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::json;

use super::super::{App, Mode};
use crate::delivery::Discard;
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
