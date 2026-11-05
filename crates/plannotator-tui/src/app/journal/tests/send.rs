//! Sends through the Inbox: `E` now, `F` the end of the round, never the agent's pane.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_journal::SendMode;
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::KeyCode;
use serde_json::json;

use super::super::super::review_test_support::{RecordingDelivery, draw};
use super::super::super::{App, Mode};
use super::press;
use crate::journal::source::thread_documents;
use crate::journal::stand_in::{StandIn, thread};

/// An Inbox message with a heading and two points, a draft comment on each point.
fn two_drafts() -> (App, Rc<StandIn>, RecordingDelivery, [String; 2]) {
    let inbox = Rc::new(StandIn::default());
    let body = "# Plan\n\nFirst point.\n\nSecond point.\n";
    inbox.state.borrow_mut().threads.push(thread("thr_1", "Plan", &[("msg_a1", true, body, json!([]))]));
    let delivery = RecordingDelivery::default();
    let found = thread_documents(inbox.as_ref(), "thr_1", false).expect("documents");
    let mut app = App::open_inbox_thread(found, inbox.clone(), 80, Box::new(delivery.clone())).expect("app");
    app.add_quote_annotation("First", Kind::Comment, "One?".into()).expect("first");
    app.add_quote_annotation("Second", Kind::Comment, "Two?".into()).expect("second");
    app.poll_inbox();
    let ids = {
        let placed = app.open.store.placed();
        [placed[0].annotation.id.clone(), placed[1].annotation.id.clone()]
    };
    (app, inbox, delivery, ids)
}

fn sends(inbox: &StandIn) -> Vec<(SendMode, Vec<String>)> {
    inbox.state.borrow().sends.iter().map(|(mode, ids, _)| (*mode, ids.clone())).collect()
}

fn header(app: &mut App) -> String {
    draw(app, 100, 12).lines().next().unwrap_or_default().trim().to_owned()
}

#[test]
fn e_sends_the_selected_notes_draft_now_and_leaves_the_round_open() {
    let (mut app, inbox, _, [first, _]) = two_drafts();
    app.select_block(1);
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(sends(&inbox), [(SendMode::Now, vec![first])]);
    assert!(!inbox.state.borrow().comments[0].draft);
    assert!(inbox.state.borrow().comments[1].draft, "the other draft waits");
    assert_eq!(header(&mut app), "round 1 \u{b7} Send 1 now \u{25b8} Inbox (E)");
}

#[test]
fn with_no_note_selected_e_sends_every_draft_waiting() {
    let (mut app, inbox, _, [first, second]) = two_drafts();
    app.select_block(0);
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(sends(&inbox), [(SendMode::Now, vec![first, second])]);
    assert_eq!(header(&mut app), "round 1 \u{b7} nothing waiting \u{25b8} Inbox");
}

#[test]
fn f_finishes_the_round_with_every_draft_left() {
    let (mut app, inbox, _, _) = two_drafts();
    press(&mut app, KeyCode::Char('F'));
    assert_eq!(sends(&inbox), [(SendMode::Round, Vec::new())]);
    assert!(inbox.state.borrow().comments.iter().all(|comment| !comment.draft));
    assert_eq!(app.status.as_deref(), Some("round 1 finished \u{2192} ses_a"));
    assert_eq!(header(&mut app), "round 2 \u{b7} nothing waiting \u{25b8} Inbox");
}

#[test]
fn an_inbox_send_never_reaches_the_agents_pane() {
    let (mut app, _, delivery, _) = two_drafts();
    app.select_block(1);
    press(&mut app, KeyCode::Char('E'));
    press(&mut app, KeyCode::Char('F'));
    assert!(delivery.calls.borrow().is_empty(), "the Inbox composes and delivers; nothing is pasted");
}

#[test]
fn a_send_tried_again_after_a_lost_connection_goes_under_the_same_key() {
    let (mut app, inbox, _, _) = two_drafts();
    inbox.state.borrow_mut().failing_sends = 1;
    press(&mut app, KeyCode::Char('E'));
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("send failed") && status.ends_with("E retry"), "{status}");
    press(&mut app, KeyCode::Char('E'));
    let keys: Vec<String> = inbox.state.borrow().sends.iter().map(|(_, _, key)| key.clone()).collect();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0], keys[1], "one press, one key, however often it is tried");
    press(&mut app, KeyCode::Char('F'));
    let keys: Vec<String> = inbox.state.borrow().sends.iter().map(|(_, _, key)| key.clone()).collect();
    assert_ne!(keys[2], keys[1], "another press is another key");
}

#[test]
fn quitting_with_drafts_waiting_asks_to_send_them_to_the_inbox() {
    let (mut app, _, _, _) = two_drafts();
    press(&mut app, KeyCode::Char('q'));
    assert_eq!(app.mode, Mode::ConfirmQuit);
    let screen = draw(&mut app, 100, 12);
    assert!(screen.contains("send feedback to the Inbox before quitting?"), "{screen}");
}

#[test]
fn sending_before_quitting_sends_every_draft_whatever_is_selected() {
    let (mut app, inbox, _, [first, second]) = two_drafts();
    app.select_block(1);
    press(&mut app, KeyCode::Char('q'));
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(sends(&inbox), [(SendMode::Now, vec![first, second])]);
    assert!(app.quit, "nothing is left waiting");
}
