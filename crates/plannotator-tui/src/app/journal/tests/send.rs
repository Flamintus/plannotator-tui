//! Sends through the journal: `E` every draft and the end of the round, pasted into the
//! agent's pane as the journal composed it; `S` the same, then closed; `F` what it was.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_journal::SendMode;
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::KeyCode;

use super::super::super::review_test_support::{Outcome, RecordingDelivery, draw};
use super::super::super::{App, Mode};
use super::{loop_app, press};
use crate::journal::stand_in::StandIn;

/// The newest reply with a draft comment on each point.
fn two_drafts() -> (App, Rc<StandIn>, RecordingDelivery, [String; 2]) {
    let (mut app, journal, delivery) = loop_app();
    app.add_quote_annotation("on-call", Kind::Comment, "Who?".into()).expect("first");
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("second");
    app.poll_journal();
    let ids = {
        let placed = app.open.store.placed();
        [placed[0].annotation.id.clone(), placed[1].annotation.id.clone()]
    };
    (app, journal, delivery, ids)
}

fn sends(journal: &StandIn) -> Vec<(SendMode, Vec<String>)> {
    journal.state.borrow().sends.iter().map(|(mode, ids, _)| (*mode, ids.clone())).collect()
}

fn header(app: &mut App) -> String {
    draw(app, 100, 12).lines().next().unwrap_or_default().trim().to_owned()
}

#[test]
fn e_sends_every_draft_ends_the_round_and_pastes_the_journals_text() {
    let (mut app, journal, delivery, [first, second]) = two_drafts();
    assert_eq!(header(&mut app), "Round 1 \u{b7} Send Feedback (2) \u{25b8} test agent");
    app.select_block(1);
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(sends(&journal), [(SendMode::Round, vec![first.clone(), second.clone()])]);
    let pasted = delivery.calls.borrow().clone();
    assert_eq!(pasted, [format!("# Comments on your work\n\nround 1: {first}, {second}")]);
    assert_eq!(journal.state.borrow().delivered, ["snd_1"], "the journal is told it was delivered");
    assert!(!app.quit, "the review stays open for the next round");
    assert_eq!(
        app.status.as_deref(),
        Some("Round 1 sent \u{b7} 2 comment(s) \u{2192} test agent \u{b7} replies will show here")
    );
    assert_eq!(header(&mut app), "Round 2 \u{b7} Sent \u{25b8} test agent");
}

#[test]
fn s_sends_and_closes() {
    let (mut app, journal, delivery, _) = two_drafts();
    press(&mut app, KeyCode::Char('S'));
    assert_eq!(sends(&journal).len(), 1);
    assert_eq!(delivery.calls.borrow().len(), 1);
    assert!(app.quit);
}

#[test]
fn f_on_a_reply_is_what_it_was() {
    let (mut app, journal, delivery, _) = two_drafts();
    press(&mut app, KeyCode::Char('F'));
    assert!(sends(&journal).is_empty() && delivery.calls.borrow().is_empty());
}

#[test]
fn with_nothing_waiting_e_says_so() {
    let (mut app, journal, delivery) = loop_app();
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(app.status.as_deref(), Some("nothing to send"));
    assert!(sends(&journal).is_empty() && delivery.calls.borrow().is_empty());
}

#[test]
fn a_paste_the_agent_did_not_get_goes_again_under_the_same_key() {
    let (mut app, journal, delivery, _) = two_drafts();
    delivery.outcome.set(Outcome::Blocked);
    press(&mut app, KeyCode::Char('E'));
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("at a dialog") && status.ends_with("E retry"), "{status}");
    assert_eq!(journal.state.borrow().delivered, Vec::<String>::new());
    assert!(header(&mut app).contains("test agent at a dialog"));
    delivery.outcome.set(Outcome::Success);
    press(&mut app, KeyCode::Char('E'));
    let keys: Vec<String> = journal.state.borrow().sends.iter().map(|(_, _, key)| key.clone()).collect();
    assert_eq!(keys.len(), 1, "the journal gave the first send back: nothing sent twice");
    let pasted = delivery.calls.borrow().clone();
    assert_eq!((pasted.len(), &pasted[0]), (2, &pasted[1]), "the same text again");
    assert_eq!(journal.state.borrow().delivered, ["snd_1"]);
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(app.status.as_deref(), Some("nothing to send"), "the retry was the press's");
}

#[test]
fn a_send_the_journal_refused_is_tried_again_under_the_same_key() {
    let (mut app, journal, delivery, _) = two_drafts();
    journal.state.borrow_mut().refused.push("send");
    press(&mut app, KeyCode::Char('E'));
    assert!(app.status.clone().unwrap_or_default().starts_with("send failed"));
    assert!(delivery.calls.borrow().is_empty());
    journal.state.borrow_mut().refused.clear();
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(sends(&journal).len(), 1);
    assert_eq!(delivery.calls.borrow().len(), 1);
}

#[test]
fn quitting_with_drafts_waiting_asks_and_yes_sends_them() {
    let (mut app, journal, _, _) = two_drafts();
    press(&mut app, KeyCode::Char('q'));
    assert_eq!(app.mode, Mode::ConfirmQuit);
    let screen = draw(&mut app, 100, 12);
    assert!(screen.contains("send feedback to test agent before quitting?"), "{screen}");
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(sends(&journal).len(), 1);
    assert!(app.quit, "nothing is left waiting");
}

#[test]
fn a_round_sent_from_one_reply_is_sent_on_the_others_too() {
    let journal = Rc::new(StandIn::default());
    let delivery = RecordingDelivery::default();
    let mut app = super::last_app(false, delivery.clone());
    app.review_in_journal(Ok(Some(journal.clone())));
    press(&mut app, KeyCode::Esc);
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("on the newest");
    app.poll_journal();
    press(&mut app, KeyCode::Char('p'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    app.add_quote_annotation("First", Kind::Comment, "Shorter.".into()).expect("on the other");
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(sends(&journal)[0].1.len(), 2, "the round carries both replies' drafts");
    press(&mut app, KeyCode::Char('p'));
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    app.poll_journal();
    assert_eq!(
        header(&mut app),
        "Round 2 \u{b7} Sent \u{25b8} test agent",
        "the reply kept aside reads its send"
    );
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(app.status.as_deref(), Some("nothing to send"));
    assert_eq!(sends(&journal).len(), 1, "no empty round");
}

#[test]
fn a_send_the_journal_refuses_for_good_is_not_kept_to_try_again() {
    let (mut app, journal, _, _) = two_drafts();
    {
        let mut state = journal.state.borrow_mut();
        state.refused.push("send");
        state.refused_as = Some("invalid");
    }
    press(&mut app, KeyCode::Char('E'));
    assert!(app.status.clone().unwrap_or_default().starts_with("send failed"));
    assert!(!app.journal_retry_waiting(), "nothing to try again: quitting does not wait on it");
}
