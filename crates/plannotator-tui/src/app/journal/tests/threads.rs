//! Threads through the journal: the agent's replies, statuses and what is new drawn in the
//! boxes under the passage, the person's replies, resolving, folding by hand, and sub-threads
//! from a thread opened as a document (from a passage selected in a box: `words`).

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_schema::{Kind, Provenance};
use ratatui::crossterm::event::{Event as Input, KeyCode};

use super::super::super::App;
use super::super::super::review_test_support::{RecordingDelivery, draw};
use super::{loop_app_of, press};
use crate::journal::stand_in::{Comment, StandIn};

/// The newest reply, a comment `c1` sent on "turns the new checkout off", and the review open
/// on it, drawn once, its block selected.
pub(super) fn commented() -> (App, Rc<StandIn>) {
    let journal = Rc::new(StandIn::default());
    let c1 = Comment::on_message("c1", "m2", "turns the new checkout off", "Who watches at night?", false);
    journal.state.borrow_mut().comments.push(c1);
    let mut app = loop_app_of(&journal, RecordingDelivery::default());
    app.poll_journal();
    draw(&mut app, 80, 30);
    app.select_block(1);
    (app, journal)
}

pub(super) fn reply_with(app: &mut App, text: &str) {
    press(app, KeyCode::Char('C'));
    app.handle_event(&Input::Paste(text.into())).expect("typed");
    press(app, KeyCode::Enter);
}

pub(super) fn row_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen.lines().find(|row| row.contains(needle)).expect(needle)
}

#[test]
fn the_agents_reply_comes_into_the_box_marked_new_once_its_journal_moves() {
    let (mut app, journal) = commented();
    assert!(!app.poll_journal(), "nothing moved");
    journal.state.borrow_mut().agent_replies("rep_1", "c1", "Nobody yet: a pager rotation.");
    assert!(app.poll_journal(), "the journal was written by another process");
    let screen = draw(&mut app, 80, 30);
    assert!(row_with(&screen, "turns the new checkout off").contains("Replied"), "{screen}");
    assert!(row_with(&screen, "Claude Code").contains("Claude Code \u{b7} Round 1 \u{b7} New"), "{screen}");
    assert!(screen.contains("Nobody yet: a pager rotation."), "{screen}");
    reply_with(&mut app, "OK for the rotation.");
    let screen = draw(&mut app, 80, 30);
    assert!(!screen.contains("\u{b7} New"), "acting on the thread sees its replies: {screen}");
}

#[test]
fn the_persons_reply_is_a_draft_that_e_sends() {
    let (mut app, journal) = commented();
    reply_with(&mut app, "And on weekends?");
    assert_eq!(app.status.as_deref(), Some("reply saved \u{b7} E sends it"));
    let reply = journal.state.borrow().comments.last().cloned().expect("a reply");
    assert_eq!(
        (reply.re.as_deref(), reply.text.as_str(), reply.draft),
        (Some("c1"), "And on weekends?", true)
    );
    let screen = draw(&mut app, 80, 30);
    assert!(row_with(&screen, "You \u{b7} Not sent").starts_with("  \u{2502}"), "{screen}");
    press(&mut app, KeyCode::Char('E'));
    let sent = journal.state.borrow().sends.last().map(|(_, ids, _)| ids.clone()).expect("a send");
    assert_eq!(sent, [reply.id]);
    let screen = draw(&mut app, 80, 30);
    assert!(screen.contains("You \u{b7} Round 1 \u{b7} Delivered"), "{screen}");
}

#[test]
fn resolve_folds_the_thread_and_opens_it_again() {
    let (mut app, journal) = commented();
    press(&mut app, KeyCode::Char('V'));
    assert!(journal.state.borrow().comments[0].resolved);
    let screen = draw(&mut app, 80, 30);
    assert!(row_with(&screen, "\u{2576} \u{2713} Resolved").contains("\u{b7} 1 message"), "{screen}");
    press(&mut app, KeyCode::Char('V'));
    assert!(!journal.state.borrow().comments[0].resolved);
    assert!(draw(&mut app, 80, 30).contains("Who watches at night?"));
}

#[test]
fn a_folded_thread_is_drawn_open_by_hand_and_folded_again() {
    let (mut app, journal) = commented();
    journal.state.borrow_mut().comments[0].resolved = true;
    journal.state.borrow_mut().written += 1;
    app.poll_journal();
    // Under 80 columns the rail is gone: what is left is the document's own column.
    assert!(!draw(&mut app, 70, 30).contains("Who watches at night?"), "folded");
    press(&mut app, KeyCode::Char('z'));
    let screen = draw(&mut app, 70, 30);
    assert!(screen.contains("Who watches at night?"), "{screen}");
    assert!(
        row_with(&screen, "\u{256d}").contains("\u{2713} Resolved"),
        "drawn open, still resolved: {screen}"
    );
    press(&mut app, KeyCode::Char('z'));
    assert!(!draw(&mut app, 70, 30).contains("Who watches at night?"), "folded again");
}

#[test]
fn a_thread_opened_as_a_document_takes_a_sub_thread_on_a_passage_of_a_reply() {
    let (mut app, journal) = commented();
    journal.state.borrow_mut().agent_replies("rep_1", "c1", "Use the admin panel. The panel is new.");
    app.poll_journal();
    press(&mut app, KeyCode::Char('O'));
    assert_eq!(app.open.source.provenance, Provenance::Stdin, "a thread is no reply of the agent's");
    assert!(app.open.doc.source.contains("Who watches at night?"));
    app.add_quote_annotation_at("panel", 2, Kind::Comment, "Which panel?".into()).expect("sub-thread");
    {
        let state = journal.state.borrow();
        let sub = state.comments.last().expect("a sub-thread");
        assert_eq!((sub.parent.as_deref(), sub.quote.as_str()), (Some("rep_1"), "panel"));
        assert_eq!(sub.quote_offset, Some(25), "the second one, counted as Plannotator counts it");
    }
    app.poll_journal();
    assert_eq!(app.open.store.placed().len(), 1, "the sub-thread is a note of the thread's document");
    press(&mut app, KeyCode::Esc);
    assert!(
        matches!(&app.open.source.provenance, Provenance::AgentMessage { message_id: Some(id), .. } if id == "m2")
    );
    assert_eq!(app.selected, 1, "back where it was left");
    app.poll_journal();
    let screen = draw(&mut app, 80, 40);
    assert!(
        row_with(&screen, "Which panel?").starts_with("  \u{2502} \u{2502}"),
        "a box in the box: {screen}"
    );
}

#[test]
fn a_comment_whose_words_are_gone_is_boxed_under_the_last_block() {
    let (mut app, journal) = commented();
    let gone = Comment::on_message("c2", "m2", "words no longer there", "Still?", false);
    journal.state.borrow_mut().comments.push(gone);
    journal.state.borrow_mut().written += 1;
    app.poll_journal();
    let screen = draw(&mut app, 80, 30);
    let rows: Vec<&str> = screen.lines().collect();
    let last = rows.iter().position(|row| row.contains("We ship on Friday.")).expect("last block");
    let head = rows.iter().position(|row| row.contains("Unanchored")).expect("marked");
    assert_eq!(head, last + 1, "{screen}");
}

#[test]
fn a_reply_that_comes_while_a_comment_is_typed_waits_and_the_comment_is_kept() {
    let (mut app, journal) = commented();
    app.select_block(2);
    press(&mut app, KeyCode::Char('c'));
    app.handle_event(&Input::Paste("Why Friday?".into())).expect("typed");
    journal.state.borrow_mut().agent_replies("rep_1", "c1", "Nobody yet: a pager rotation.");
    assert!(!app.poll_journal(), "nothing is read while the person types");
    draw(&mut app, 80, 30);
    press(&mut app, KeyCode::Enter);
    let texts: Vec<String> = journal.state.borrow().comments.iter().map(|c| c.text.clone()).collect();
    assert!(texts.contains(&"Why Friday?".to_owned()), "{texts:?}");
    assert!(app.poll_journal(), "what came is read once the comment is saved");
    assert!(draw(&mut app, 80, 30).contains("Nobody yet: a pager rotation."));
}

#[test]
fn a_selection_waiting_for_its_action_survives_a_box_growing_above_it() {
    let (mut app, journal) = commented();
    app.select_quote_for_snapshot("on Friday").expect("selected");
    journal.state.borrow_mut().agent_replies("rep_1", "c1", "Nobody yet: a pager rotation.");
    assert!(app.poll_journal());
    draw(&mut app, 80, 30);
    press(&mut app, KeyCode::Char('c'));
    app.handle_event(&Input::Paste("Why Friday?".into())).expect("typed");
    press(&mut app, KeyCode::Enter);
    let state = journal.state.borrow();
    let made = state.comments.iter().find(|c| c.text == "Why Friday?").expect("the comment");
    assert_eq!(made.quote, "on Friday");
}

#[test]
fn a_read_that_failed_is_tried_again_after_a_pause_and_misses_no_reply() {
    let (mut app, journal) = commented();
    journal.state.borrow_mut().refused.push("view");
    journal.state.borrow_mut().agent_replies("rep_1", "c1", "Nobody yet: a pager rotation.");
    assert!(app.poll_journal(), "the journal moved: read");
    assert!(app.status.clone().unwrap_or_default().starts_with("review loop:"), "{:?}", app.status);
    journal.state.borrow_mut().refused.clear();
    assert!(!app.poll_journal(), "not on every wake");
    let review_loop = app.review_loop.as_mut().expect("the loop runs");
    review_loop.failed = review_loop.failed.and_then(|failed| failed.checked_sub(super::super::RETRY));
    assert!(app.poll_journal(), "after the pause");
    assert!(draw(&mut app, 80, 30).contains("Nobody yet: a pager rotation."));
}

#[test]
fn a_note_not_saved_in_a_thread_survives_a_reply_rewriting_the_thread() {
    let (mut app, journal) = commented();
    journal.state.borrow_mut().agent_replies("rep_1", "c1", "Use the admin panel.");
    app.poll_journal();
    press(&mut app, KeyCode::Char('O'));
    journal.state.borrow_mut().refused.push("comment");
    app.add_quote_annotation("admin panel", Kind::Comment, "Which one?".into()).expect("sub-thread");
    app.poll_journal();
    journal.state.borrow_mut().agent_replies("rep_2", "c1", "And the runbook.");
    app.poll_journal();
    assert!(app.open.doc.source.contains("And the runbook."), "the thread was written again");
    let kept: Vec<String> = app.open.store.placed().iter().map(|p| p.annotation.body.clone()).collect();
    assert_eq!(kept, ["Which one?"], "the person's words are not lost");
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert_eq!(app.journal_tag(&id), " \u{b7} Not saved");
}

#[test]
fn a_folded_threads_bubble_counts_its_replies_rather_than_repeat_them() {
    let (mut app, journal) = commented();
    journal.state.borrow_mut().agent_replies("rep_1", "c1", "Nobody yet: a pager rotation.");
    journal.state.borrow_mut().comments[0].resolved = true;
    app.poll_journal();
    let screen = draw(&mut app, 110, 30);
    assert!(screen.contains("\u{2576} \u{2713} Resolved \u{b7}"), "folded: {screen}");
    assert!(screen.contains("\u{21b3} 1 reply under the passage"), "{screen}");
    assert!(!screen.contains("Nobody yet"), "nor in the bubble: {screen}");
}
