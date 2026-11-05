//! Threads through the Inbox: replies, statuses and what is new drawn in the boxes under
//! the passage, resolving, folding by hand, and sub-threads from a thread opened as a
//! document.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_inbox::{Event, Kind as Record, Record as Line, Status};
use plannotator_tui_schema::{Kind, Provenance};
use ratatui::crossterm::event::{Event as Input, KeyCode};

use super::super::super::App;
use super::super::super::review_test_support::{draw, file_app};
use super::{press, thread_app_of};
use crate::inbox::stand_in::{Comment, StandIn, thread};

const BODY: &str = "# Rollout\n\nThe on-call engineer turns the new checkout off.\n\nWe ship on Friday.\n";

/// An Inbox message, a comment `c1` sent on "turns the new checkout off", and the review
/// open on it, drawn once.
fn commented(live: bool) -> (App, Rc<StandIn>) {
    let inbox = Rc::new(StandIn::default());
    {
        let mut state = inbox.state.borrow_mut();
        state.threads.push(thread("thr_1", "Rollout", &[("msg_a1", true, BODY, serde_json::json!([]))]));
        let mut c1 =
            Comment::on_message("c1", "msg_a1", "turns the new checkout off", "Who watches at night?", false);
        c1.live = live;
        state.comments.push(c1);
    }
    let mut app = thread_app_of(&inbox);
    app.poll_inbox();
    draw(&mut app, 80, 30);
    app.select_block(1);
    (app, inbox)
}

fn agent_replies(inbox: &StandIn, id: &str, text: &str) {
    let mut state = inbox.state.borrow_mut();
    let reply = Comment::agent_reply(id, &state.comments[0], text);
    state.comments[0].status = Some(Status::Replied);
    state.comments.push(reply);
    let line = Line { seq: 9, kind: Record::CommentReply, id: id.into(), thread_id: Some("thr_1".into()) };
    state.stream.as_ref().expect("followed").send(Event::Records(vec![line])).expect("listened");
}

fn reply_with(app: &mut App, text: &str) {
    press(app, KeyCode::Char('C'));
    app.handle_event(&Input::Paste(text.into())).expect("typed");
    press(app, KeyCode::Enter);
}

fn row_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen.lines().find(|row| row.contains(needle)).expect(needle)
}

#[test]
fn the_agents_reply_comes_into_the_box_marked_new_and_the_send_says_it_was_answered() {
    let (mut app, inbox) = commented(false);
    agent_replies(&inbox, "rep_1", "Nobody yet: a pager rotation.");
    assert!(app.poll_inbox());
    let screen = draw(&mut app, 80, 30);
    assert!(row_with(&screen, "turns the new checkout off").contains("replied"), "{screen}");
    let author = row_with(&screen, "Claude Code");
    assert!(author.contains("Claude Code \u{b7} new"), "{screen}");
    assert!(screen.contains("Nobody yet: a pager rotation."), "{screen}");
    reply_with(&mut app, "OK for the rotation.");
    let screen = draw(&mut app, 80, 30);
    assert!(!screen.contains("\u{b7} new"), "acting on the thread sees its replies: {screen}");
}

#[test]
fn the_persons_reply_is_a_draft_that_e_sends() {
    let (mut app, inbox) = commented(false);
    reply_with(&mut app, "And on weekends?");
    assert_eq!(app.status.as_deref(), Some("reply saved \u{b7} E sends it"));
    let reply = inbox.state.borrow().comments.last().cloned().expect("a reply");
    assert_eq!(
        (reply.re.as_deref(), reply.text.as_str(), reply.draft),
        (Some("c1"), "And on weekends?", true)
    );
    let screen = draw(&mut app, 80, 30);
    assert!(row_with(&screen, "You \u{b7} draft").starts_with("  \u{2502}"), "{screen}");
    press(&mut app, KeyCode::Char('E'));
    let sent = inbox.state.borrow().sends.last().map(|(_, ids, _)| ids.clone()).expect("a send");
    assert_eq!(sent, [reply.id]);
    let screen = draw(&mut app, 80, 30);
    assert!(screen.contains("You \u{b7} pending"), "{screen}");
}

#[test]
fn a_reply_under_a_live_thread_goes_on_its_own() {
    let (mut app, inbox) = commented(true);
    reply_with(&mut app, "And on weekends?");
    assert_eq!(app.status.as_deref(), Some("reply sent: the thread is live"));
    assert!(!inbox.state.borrow().comments.last().expect("a reply").draft);
    assert!(inbox.state.borrow().sends.is_empty(), "the Inbox sends it; no press does");
}

#[test]
fn resolve_folds_the_thread_and_opens_it_again() {
    let (mut app, inbox) = commented(false);
    press(&mut app, KeyCode::Char('V'));
    assert!(inbox.state.borrow().comments[0].resolved);
    let screen = draw(&mut app, 80, 30);
    assert!(row_with(&screen, "\u{2713}").contains("resolved \u{b7} 1 message"), "{screen}");
    press(&mut app, KeyCode::Char('V'));
    assert!(!inbox.state.borrow().comments[0].resolved);
    let screen = draw(&mut app, 80, 30);
    assert!(screen.contains("Who watches at night?"), "{screen}");
}

#[test]
fn a_folded_thread_is_drawn_open_by_hand_and_folded_again() {
    let (mut app, inbox) = commented(false);
    inbox.state.borrow_mut().comments[0].resolved = true;
    app.open.store.mark_inbox_stale();
    app.poll_inbox();
    // Under 80 columns the rail is gone: what is left is the document's own column.
    assert!(!draw(&mut app, 70, 30).contains("Who watches at night?"), "folded");
    press(&mut app, KeyCode::Char('z'));
    let screen = draw(&mut app, 70, 30);
    assert!(screen.contains("Who watches at night?"), "{screen}");
    assert!(row_with(&screen, "\u{256d}").contains("resolved"), "drawn open, still resolved: {screen}");
    press(&mut app, KeyCode::Char('z'));
    assert!(!draw(&mut app, 70, 30).contains("Who watches at night?"), "folded again");
}

#[test]
fn a_thread_opened_as_a_document_takes_a_sub_thread_on_a_passage_of_a_reply() {
    let (mut app, inbox) = commented(false);
    agent_replies(&inbox, "rep_1", "Use the admin panel. The panel is new.");
    app.poll_inbox();
    press(&mut app, KeyCode::Char('O'));
    assert_eq!(
        app.open.source.provenance,
        Provenance::Inbox { thread_id: "thr_1".into(), message_id: "c1".into(), attachment_id: None }
    );
    assert!(app.open.doc.source.contains("Who watches at night?"));
    app.add_quote_annotation_at("panel", 2, Kind::Comment, "Which panel?".into()).expect("sub-thread");
    let passage = inbox.state.borrow().passages.last().cloned().expect("a passage");
    assert_eq!((passage.parent.as_str(), passage.quote.as_str(), passage.occurrence), ("rep_1", "panel", 1));
    app.poll_inbox();
    assert_eq!(app.open.store.placed().len(), 1, "the sub-thread is a note of the thread's document");
    press(&mut app, KeyCode::Esc);
    assert_eq!(
        app.open.source.provenance,
        Provenance::Inbox { thread_id: "thr_1".into(), message_id: "msg_a1".into(), attachment_id: None }
    );
    assert_eq!(app.selected, 1, "back where it was left");
    app.poll_inbox();
    let screen = draw(&mut app, 80, 40);
    assert!(
        row_with(&screen, "Which panel?").starts_with("  \u{2502} \u{2502}"),
        "a box in the box: {screen}"
    );
}

#[test]
fn a_comment_whose_words_are_gone_is_boxed_under_the_last_block() {
    let (mut app, inbox) = commented(false);
    let gone = Comment::on_message("c2", "msg_a1", "words no longer there", "Still?", false);
    inbox.state.borrow_mut().comments.push(gone);
    app.open.store.mark_inbox_stale();
    app.poll_inbox();
    let screen = draw(&mut app, 80, 30);
    let rows: Vec<&str> = screen.lines().collect();
    let last = rows.iter().position(|row| row.contains("We ship on Friday.")).expect("last block");
    let head = rows.iter().position(|row| row.contains("not found")).expect("marked");
    assert_eq!(head, last + 1, "{screen}");
}

#[test]
fn without_the_inbox_the_thread_keys_say_why() {
    let (root, mut app, _) = file_app("thread-keys-off");
    app.add_quote_annotation("two", Kind::Comment, "Why?".into()).expect("note");
    app.select_block(2);
    press(&mut app, KeyCode::Char('C'));
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("go through the Inbox"), "{status}");
    std::fs::remove_dir_all(root).expect("cleanup");
}
