//! Sub-threads on a passage selected in the words of a box, by mouse or by keyboard: on one
//! sent message's words only, with the passage's words and offset as Plannotator counts them;
//! the thread keys on the box the cursor is in; a passage waiting for its action across a
//! change of the boxes.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use ratatui::crossterm::event::{Event as Input, KeyCode};

use super::super::super::review_test_support::{drag, drag_over, draw, on_screen};
use super::super::super::{App, Mode};
use super::press;
use super::threads::{commented, reply_with, row_with};
use crate::journal::stand_in::{Comment, StandIn};

/// `commented`, the agent's reply `text` under c1, drawn once.
fn answered(text: &str) -> (App, Rc<StandIn>) {
    let (mut app, journal) = commented();
    journal.state.borrow_mut().agent_replies("rep_1", "c1", text);
    app.poll_journal();
    draw(&mut app, 80, 30);
    (app, journal)
}

fn sub_thread_with(app: &mut App, text: &str) {
    press(app, KeyCode::Char('c'));
    app.handle_event(&Input::Paste(text.into())).expect("typed");
    press(app, KeyCode::Enter);
}

/// The sub-threads the journal was given: `(parent, quote, offset)`.
fn sub_threads(journal: &StandIn) -> Vec<(Option<String>, String, Option<u64>)> {
    let state = journal.state.borrow();
    let subs = state.comments.iter().filter(|comment| comment.parent.is_some());
    subs.map(|sub| (sub.parent.clone(), sub.quote.clone(), sub.quote_offset)).collect()
}

#[test]
fn a_passage_selected_in_a_reply_in_its_box_takes_a_sub_thread_on_it() {
    let (mut app, journal) = answered("Use the admin panel. The panel is new.");
    let screen = draw(&mut app, 80, 30);
    drag_over(&mut app, &screen, "panel", 1);
    let footer = draw(&mut app, 80, 30).lines().last().unwrap_or_default().to_owned();
    assert!(footer.contains("c comment") && !footer.contains("looks good"), "a comment alone: {footer}");
    sub_thread_with(&mut app, "Which panel?");
    // The second one, counted as Plannotator counts it.
    assert_eq!(sub_threads(&journal), [(Some("rep_1".into()), "panel".into(), Some(25))]);
    assert!(journal.state.borrow().comments.last().is_some_and(|sub| sub.draft), "a draft");
    assert_eq!(app.status.as_deref(), Some("sub-thread saved \u{b7} E sends it"));
    let screen = draw(&mut app, 80, 40);
    let row = row_with(&screen, "Which panel?");
    assert!(row.starts_with("  \u{2502} \u{2502}"), "a box in the box: {screen}");
}

#[test]
fn a_passage_after_tabs_and_wide_characters_keeps_its_words_and_offset() {
    let (mut app, journal) = answered("Use\tthe \u{30d1}\u{30cd}\u{30eb} panel.");
    let screen = draw(&mut app, 80, 30);
    let (x, y) = on_screen(&screen, "panel", 0);
    drag(&mut app, on_screen(&screen, "\u{30d1}", 0), (x + 4, y));
    sub_thread_with(&mut app, "Which one?");
    let expected = (Some("rep_1".into()), "\u{30d1}\u{30cd}\u{30eb} panel".into(), Some(8));
    assert_eq!(sub_threads(&journal), [expected]);
}

#[test]
fn the_keyboard_reaches_the_words_in_a_box_and_selects_in_them() {
    let (mut app, journal) = answered("Use the admin panel.");
    app.select_block(1);
    // Down from the paragraph, over the box's head and author lines, to the comment's words,
    // then the reply's: the cursor stands on the words, never on the box's side.
    for code in "ijj".chars().map(KeyCode::Char) {
        press(&mut app, code);
    }
    assert_eq!(app.cursor.1, 2, "past the box's side");
    // `w` twice to "admin", `v`, four columns on.
    for code in "wwvllll".chars().map(KeyCode::Char).chain([KeyCode::Enter]) {
        press(&mut app, code);
    }
    sub_thread_with(&mut app, "Which admin?");
    assert_eq!(sub_threads(&journal), [(Some("rep_1".into()), "admin".into(), Some(8))]);
}

#[test]
fn comment_in_a_boxs_words_asks_for_a_passage_first() {
    let (mut app, journal) = answered("Use the admin panel.");
    app.select_block(1);
    for code in "ijjc".chars().map(KeyCode::Char) {
        press(&mut app, code);
    }
    assert_eq!(app.mode, Mode::Browse, "no comment on the block above");
    assert_eq!(
        app.status.as_deref(),
        Some("v selects a passage of this message, then c starts a sub-thread on it")
    );
    assert_eq!(sub_threads(&journal), []);
}

#[test]
fn a_sub_thread_is_on_one_messages_words() {
    let (mut app, journal) = answered("Use the admin panel.");
    let screen = draw(&mut app, 80, 30);
    // The comment's words are in its bubble too, on the right: the box's are the second.
    drag(&mut app, on_screen(&screen, "night?", 1), on_screen(&screen, "admin", 0));
    assert_eq!(app.status.as_deref(), Some("a sub-thread is on one message's words: select within one"));
    assert!(app.pending.is_none() && sub_threads(&journal).is_empty());
}

#[test]
fn a_drafts_words_take_no_sub_thread() {
    let (mut app, journal) = answered("Use the admin panel.");
    reply_with(&mut app, "And the runbook?");
    let screen = draw(&mut app, 80, 30);
    drag_over(&mut app, &screen, "runbook", 0);
    assert_eq!(app.status.as_deref(), Some("a draft takes no sub-thread: E sends it first"));
    assert!(app.pending.is_none() && sub_threads(&journal).is_empty());
}

#[test]
fn the_thread_keys_act_on_the_box_the_cursor_is_in() {
    let (mut app, journal) = answered("Use the admin panel.");
    {
        let mut state = journal.state.borrow_mut();
        // Its passage starts after c1's: the block's first note is c1.
        state.comments.push(Comment::on_message("c2", "m2", "checkout off", "Off how?", false));
        state.agent_replies("rep_2", "c2", "Bob flips the switch.");
    }
    app.poll_journal();
    let screen = draw(&mut app, 80, 40);
    let bob = on_screen(&screen, "Bob", 0);
    drag(&mut app, bob, bob);
    press(&mut app, KeyCode::Char('V'));
    let state = journal.state.borrow();
    let resolved = |id: &str| state.comments.iter().any(|comment| comment.id == id && comment.resolved);
    assert!(resolved("c2") && !resolved("c1"), "the box clicked in, not the block's first: {screen}");
}

#[test]
fn a_waiting_passage_follows_its_words_and_goes_when_they_fold() {
    let (mut app, journal) = answered("Use the admin panel.");
    let screen = draw(&mut app, 80, 30);
    drag_over(&mut app, &screen, "admin", 0);
    // A reply above the passage moves it down: the toolbar goes with it.
    let before = app.pending.as_ref().map(|pending| pending.at).expect("waiting");
    {
        let mut state = journal.state.borrow_mut();
        state.comments.push(Comment::on_message("c0", "m2", "Rollout", "Which rollout?", false));
        state.agent_replies("rep_0", "c0", "This one.");
    }
    app.poll_journal();
    let screen = draw(&mut app, 80, 40);
    let after = app.pending.as_ref().map(|pending| pending.at).expect("still waiting");
    let (_, row) = on_screen(&screen, "admin", 0);
    let row = usize::from(row - app.geometry.doc.y) + app.scroll;
    assert!(after.0 > before.0 && after.0 == row, "{before:?} {after:?}: {screen}");
    // Resolved, the thread folds: its words are gone, and the passage with them.
    press(&mut app, KeyCode::Char('V'));
    draw(&mut app, 80, 40);
    assert!(app.pending.is_none(), "{:?}", app.pending);
}
