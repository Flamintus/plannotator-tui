#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use plannotator_tui_schema::Provenance;
use ratatui::crossterm::event::KeyCode;

use super::super::super::Mode;
use super::{press, thread_app};

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
