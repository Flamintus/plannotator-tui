//! Comments kept by the Inbox: written one action at a time, read back from its view.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_journal::{Anchor as On, FileVersion};
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::KeyCode;
use serde_json::json;

use super::super::super::App;
use super::{press, thread_app_of, thread_inbox};
use crate::journal::stand_in::{Comment, StandIn, thread};

/// `inbox`'s thread with its newest message open.
fn message_app(inbox: &Rc<StandIn>) -> App {
    let mut app = thread_app_of(inbox);
    press(&mut app, KeyCode::Enter);
    app
}

fn writes(inbox: &StandIn) -> Vec<String> {
    inbox.state.borrow().writes.clone()
}

fn placed_words(app: &App) -> Vec<String> {
    let placed = app.open.store.placed();
    placed.iter().filter_map(|p| app.open.doc.source.get(p.range.clone())).map(str::to_owned).collect()
}

#[test]
fn a_comment_is_one_write_and_comes_back_as_the_inbox_lists_it() {
    let inbox = thread_inbox();
    let mut app = message_app(&inbox);
    app.add_quote_annotation("answer", Kind::Comment, "Which one?".into()).expect("comment");
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert_eq!(writes(&inbox), [format!("comment {id}")]);
    {
        let state = inbox.state.borrow();
        let kept = &state.comments[0];
        assert_eq!((kept.quote.as_str(), kept.text.as_str(), kept.draft), ("answer", "Which one?", true));
        assert_eq!(kept.on, On::Message { message_id: "msg_a2".into() });
    }
    assert!(app.poll_inbox(), "a write has the view read again");
    assert!(!app.poll_inbox(), "and only once");
    assert_eq!(placed_words(&app), ["answer"]);
}

#[test]
fn a_drafts_edit_and_its_removal_are_one_write_each() {
    let inbox = thread_inbox();
    let mut app = message_app(&inbox);
    app.add_quote_annotation("answer", Kind::Comment, "Which one?".into()).expect("comment");
    app.poll_inbox();
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert!(app.open.store.edit_body(&id, "Which answer?".into()).expect("edit"));
    app.poll_inbox();
    assert_eq!(inbox.state.borrow().comments[0].text, "Which answer?");
    assert!(app.open.store.remove(&id).expect("remove"));
    app.poll_inbox();
    assert!(inbox.state.borrow().comments.is_empty());
    assert_eq!(writes(&inbox), [format!("comment {id}"), format!("comment {id}"), format!("remove {id}")]);
    assert!(app.open.store.placed().is_empty());
}

#[test]
fn a_sent_comment_is_neither_edited_nor_removed_and_says_why() {
    let inbox = thread_inbox();
    inbox.state.borrow_mut().comments.push(Comment::on_message(
        "c1",
        "msg_a2",
        "Second",
        "Sent words.",
        false,
    ));
    let mut app = message_app(&inbox);
    assert!(app.open.store.edit_body("c1", "Other words.".into()).expect("edit"));
    app.poll_inbox();
    assert!(writes(&inbox).is_empty(), "nothing is written over a sent comment");
    assert_eq!(app.open.store.placed()[0].annotation.body, "Sent words.", "the Inbox's words are back");
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("c1 is sent"), "{status}");
    assert!(app.open.store.remove("c1").expect("remove"));
    app.poll_inbox();
    assert_eq!(placed_words(&app), ["Second"], "a sent comment stays");
}

#[test]
fn a_comment_made_elsewhere_is_placed_on_its_words_as_drawn() {
    let inbox = Rc::new(StandIn::default());
    {
        let mut state = inbox.state.borrow_mut();
        let body = "Ship the **login page** by Friday.";
        state.threads.push(thread("thr_1", "Plan", &[("msg_a1", true, body, json!([]))]));
        state.comments.push(Comment::on_message("c1", "msg_a1", "login page by", "Why Friday?", false));
    }
    let app = thread_app_of(&inbox);
    assert_eq!(placed_words(&app), ["login page** by"]);
}

#[test]
fn a_comment_on_an_attached_file_names_the_file_and_its_version() {
    let inbox = thread_inbox();
    let mut app = thread_app_of(&inbox);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    app.add_quote_annotation("Ship it.", Kind::Comment, "When?".into()).expect("comment");
    let on = inbox.state.borrow().comments[0].on.clone();
    assert_eq!(
        on,
        On::File { path: "/repo/plan.md".into(), version: FileVersion::Current, lines: None, at: None }
    );
}

#[test]
fn an_inbox_that_cannot_list_comments_leaves_the_notes_here_and_says_so() {
    let inbox = thread_inbox();
    inbox.state.borrow_mut().missing.push("threads/:id/comments");
    let mut app = message_app(&inbox);
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("notes on this document stay here"), "{status}");
    app.add_quote_annotation("answer", Kind::Comment, "Which one?".into()).expect("comment");
    assert!(writes(&inbox).is_empty(), "nothing reaches an Inbox that cannot list it");
    assert!(!app.poll_inbox());
}

#[test]
fn a_note_the_inbox_did_not_take_stays_here_marked_until_a_write_takes_it() {
    let inbox = thread_inbox();
    let mut app = message_app(&inbox);
    inbox.state.borrow_mut().missing.push("messages/:id/annotations");
    app.add_quote_annotation("answer", Kind::Comment, "Which one?".into()).expect("comment");
    app.poll_inbox();
    assert_eq!(placed_words(&app), ["answer"], "its words are not lost");
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert_eq!(app.inbox_tag(&id), " \u{b7} not saved");
    inbox.state.borrow_mut().missing.clear();
    assert!(app.open.store.edit_body(&id, "Which answer?".into()).expect("edit"));
    app.poll_inbox();
    assert_eq!(inbox.state.borrow().comments[0].text, "Which answer?");
    assert_eq!(app.inbox_tag(&id), " \u{b7} draft");
}
