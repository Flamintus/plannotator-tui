//! The review's Live mode through the journal: `ctrl+l` turns it on at once with no draft
//! waiting, asks first with some (only `y` sends them, then turns it on), and off at once; while
//! it is on a new note goes as soon as it is saved, every sent thread is Live, and one turned
//! Unlive stays out of it until the mode comes on again. The header and the footer say it.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use plannotator_tui_journal::SendMode;
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::super::super::review_test_support::draw;
use super::super::super::{App, Mode};
use super::threads::{commented, reply_with};
use super::{loop_app, press, writes};
use crate::journal::stand_in::StandIn;

fn ctrl_l(app: &mut App) {
    app.handle_event(&Event::Key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL))).expect("key");
}

fn sends(journal: &StandIn) -> Vec<(SendMode, Vec<String>)> {
    journal.state.borrow().sends.iter().map(|(mode, ids, _)| (*mode, ids.clone())).collect()
}

fn header(app: &mut App) -> String {
    draw(app, 100, 12).lines().next().unwrap_or_default().to_owned()
}

fn footer(app: &mut App) -> String {
    draw(app, 140, 12).lines().last().unwrap_or_default().to_owned()
}

/// Whether thread `id` is Live in the view the open document last read.
fn live(app: &App, id: &str) -> bool {
    let (_, view) = app.open.store.journal_view().expect("the journal's view");
    view.threads.iter().find(|thread| thread.id == id).expect(id).live
}

#[test]
fn ctrl_l_with_no_draft_turns_the_live_mode_on_and_off_at_once_and_the_header_says_it() {
    let (mut app, journal, _) = loop_app();
    assert!(footer(&mut app).contains("live mode"), "the footer offers it");
    assert!(!header(&mut app).contains("Live mode"));
    ctrl_l(&mut app);
    assert_eq!(writes(&journal), ["live-mode true"]);
    assert_eq!(app.mode, Mode::Browse, "nothing to ask: no draft waits");
    let head = header(&mut app);
    // The lightning is two columns wide: the screen's text has a blank cell after it.
    let live_at = head.find("\u{26a1}  Live mode").expect("the header says it");
    assert!(
        live_at < head.find("Round").expect("the send button"),
        "left of the buttons it stands before: {head}"
    );
    assert!(footer(&mut app).contains("unlive mode"));
    ctrl_l(&mut app);
    assert_eq!(writes(&journal), ["live-mode true", "live-mode false"]);
    assert_eq!(app.status.as_deref(), Some("live mode off"));
    assert!(!header(&mut app).contains("Live mode"));
}

#[test]
fn ctrl_l_with_drafts_asks_first_and_only_yes_sends_them_before_the_live_mode() {
    let (mut app, journal, delivery) = loop_app();
    app.add_quote_annotation("on-call", Kind::Comment, "Who?".into()).expect("first");
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("second");
    app.poll_journal();
    let ids: Vec<String> =
        app.open.store.placed().iter().map(|placed| placed.annotation.id.clone()).collect();

    ctrl_l(&mut app);
    assert_eq!(app.mode, Mode::ConfirmLiveOn);
    assert!(footer(&mut app).contains("2 draft(s) go now"), "{}", footer(&mut app));
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(sends(&journal), [], "no answer, no send");
    assert!(!writes(&journal).iter().any(|write| write.starts_with("live-mode")));

    ctrl_l(&mut app);
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(sends(&journal), [(SendMode::Now, ids)], "every draft at once, the round going on");
    assert_eq!(delivery.calls.borrow().len(), 1, "pasted into the agent's pane as a send is");
    assert_eq!(
        writes(&journal).last().map(String::as_str),
        Some("live-mode true"),
        "the mode after the drafts"
    );
    let status = app.status.clone().unwrap_or_default();
    assert!(
        status.starts_with("Sent now")
            && status.ends_with("live mode on: every thread live, new notes sent as saved"),
        "{status}"
    );
}

#[test]
fn a_new_note_goes_at_once_while_the_live_mode_is_on() {
    let (mut app, journal, _) = loop_app();
    ctrl_l(&mut app);
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("note");
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert_eq!(sends(&journal), [(SendMode::Now, vec![id])], "sent as it was saved, not left a draft");
    assert!(app.status.as_deref().is_some_and(|status| status.starts_with("Sent now")), "{:?}", app.status);
}

#[test]
fn a_thread_turned_unlive_under_the_live_mode_stays_out_until_the_mode_comes_on_again() {
    let (mut app, journal) = commented();
    assert!(!live(&app, "c1"));
    ctrl_l(&mut app);
    assert!(live(&app, "c1"), "every sent thread is Live in the Live mode");
    // `L` in the box turns this one Unlive: a reply under it then waits as a draft.
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('L'));
    assert_eq!(writes(&journal).last().map(String::as_str), Some("live c1 false"));
    assert!(!live(&app, "c1"));
    reply_with(&mut app, "And on weekends?");
    assert_eq!(app.status.as_deref(), Some("reply saved"));
    assert_eq!(sends(&journal), []);
    // Off, it stays as it was set; on again, every thread is Live again.
    ctrl_l(&mut app);
    assert!(!live(&app, "c1"));
    ctrl_l(&mut app);
    assert_eq!(app.mode, Mode::ConfirmLiveOn, "the reply waiting is asked about first");
    press(&mut app, KeyCode::Char('y'));
    assert!(live(&app, "c1"), "the mode on again makes it Live again");
}
