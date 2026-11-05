//! `last` with the review loop off, on, or unable to run.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_journal::{Anchor as On, ReviewTarget};
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::KeyCode;

use super::super::super::Mode;
use super::super::super::review_test_support::RecordingDelivery;
use super::{last_app, loop_app, loop_app_of, press, writes};
use crate::journal::stand_in::StandIn;

#[test]
fn with_the_loop_off_last_is_what_it_always_was() {
    let delivery = RecordingDelivery::default();
    let mut app = last_app(true, delivery.clone());
    app.review_in_journal(Ok(None));
    assert!(!app.open.store.is_journal());
    assert_eq!(app.status, None, "nothing said");
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("note");
    app.select_block(2);
    for key in ['C', 'V', 'O', 'z'] {
        press(&mut app, KeyCode::Char(key));
    }
    assert_eq!((app.mode == Mode::Browse, app.status.as_deref()), (true, None), "the thread keys are inert");
    press(&mut app, KeyCode::Char('E'));
    let pasted = delivery.calls.borrow().clone();
    assert_eq!(pasted.len(), 1);
    assert!(pasted[0].starts_with("# Annotations on claude"), "today's feedback: {pasted:?}");
}

#[test]
fn the_open_reply_is_kept_in_its_sessions_review() {
    let (app, journal, _) = loop_app();
    assert!(app.open.store.is_journal());
    let named = journal.state.borrow().targets.last().cloned().expect("a call");
    assert_eq!(named, ReviewTarget::Last { host: "claude-code".into(), session: "ses_a".into() });
    assert_eq!(app.status, None);
}

#[test]
fn a_reply_picked_later_is_kept_in_the_same_review() {
    let journal = Rc::new(StandIn::default());
    let mut app = last_app(false, RecordingDelivery::default());
    app.review_in_journal(Ok(Some(journal.clone())));
    assert_eq!(app.mode, Mode::Pick);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.open.doc.source, "First reply.\n");
    app.add_quote_annotation("First", Kind::Comment, "Shorter.".into()).expect("note");
    assert_eq!(journal.state.borrow().comments[0].on, On::Message { message_id: "m1".into() });
}

#[test]
fn a_journal_that_cannot_list_the_comments_leaves_the_notes_here_and_says_so() {
    let journal = Rc::new(StandIn::default());
    journal.state.borrow_mut().refused.push("view");
    let delivery = RecordingDelivery::default();
    let mut app = loop_app_of(&journal, delivery.clone());
    assert!(!app.open.store.is_journal());
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("notes on this reply stay here"), "{status}");
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("note");
    assert!(writes(&journal).is_empty(), "nothing reaches a journal that cannot list it");
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(delivery.calls.borrow().len(), 1, "its feedback goes to the agent as before");
}

#[test]
fn a_loop_that_cannot_run_says_why_and_last_goes_on_as_before() {
    let mut app = last_app(true, RecordingDelivery::default());
    app.review_in_journal(Err(anyhow::anyhow!("no `plannotator` on the PATH")));
    assert!(!app.open.store.is_journal());
    let status = app.status.clone().unwrap_or_default();
    assert_eq!(
        status,
        "review loop off: no `plannotator` on the PATH \u{b7} feedback goes to the agent as before"
    );
}
