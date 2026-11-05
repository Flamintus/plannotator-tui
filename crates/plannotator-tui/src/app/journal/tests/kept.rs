//! Comments kept by the journal: written one action at a time, read back from its view.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_journal::Anchor as On;
use plannotator_tui_schema::Kind;

use super::super::super::App;
use super::super::super::review_test_support::RecordingDelivery;
use super::{last_app_with, loop_app, writes};
use crate::journal::stand_in::{Comment, StandIn};

fn placed_words(app: &App) -> Vec<String> {
    let placed = app.open.store.placed();
    placed.iter().filter_map(|p| app.open.doc.source.get(p.range.clone())).map(str::to_owned).collect()
}

#[test]
fn a_comment_is_one_write_and_comes_back_as_the_journal_lists_it() {
    let (mut app, journal, _) = loop_app();
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("comment");
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert_eq!(writes(&journal), [format!("comment {id}")]);
    {
        let state = journal.state.borrow();
        let kept = &state.comments[0];
        assert_eq!((kept.quote.as_str(), kept.text.as_str(), kept.draft), ("We ship", "Why Friday?", true));
        assert_eq!(kept.on, On::Message { message_id: "m2".into() });
    }
    assert!(app.poll_journal(), "a write has the view read again");
    assert!(!app.poll_journal(), "and only once");
    assert_eq!(placed_words(&app), ["We ship"]);
}

#[test]
fn a_drafts_edit_and_its_removal_are_one_write_each() {
    let (mut app, journal, _) = loop_app();
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("comment");
    app.poll_journal();
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert!(app.open.store.edit_body(&id, "Why not Monday?".into()).expect("edit"));
    app.poll_journal();
    assert_eq!(journal.state.borrow().comments[0].text, "Why not Monday?");
    assert!(app.open.store.remove(&id).expect("remove"));
    app.poll_journal();
    assert!(journal.state.borrow().comments.is_empty());
    assert_eq!(writes(&journal), [format!("comment {id}"), format!("comment {id}"), format!("remove {id}")]);
    assert!(app.open.store.placed().is_empty());
}

#[test]
fn a_sent_comment_is_neither_edited_nor_removed_and_says_why() {
    let journal = Rc::new(StandIn::default());
    journal.state.borrow_mut().comments.push(Comment::on_message(
        "c1",
        "m2",
        "We ship",
        "Sent words.",
        false,
    ));
    let mut app = super::loop_app_of(&journal, RecordingDelivery::default());
    assert!(app.open.store.edit_body("c1", "Other words.".into()).expect("edit"));
    app.poll_journal();
    assert!(writes(&journal).is_empty(), "nothing is written over a sent comment");
    assert_eq!(app.open.store.placed()[0].annotation.body, "Sent words.", "the journal's words are back");
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("c1 is sent"), "{status}");
    assert!(app.open.store.remove("c1").expect("remove"));
    app.poll_journal();
    assert_eq!(placed_words(&app), ["We ship"], "a sent comment stays");
}

#[test]
fn a_comment_made_elsewhere_is_placed_on_its_words_as_drawn() {
    let journal = Rc::new(StandIn::default());
    journal.state.borrow_mut().comments.push(Comment::on_message(
        "c1",
        "m3",
        "login page by",
        "Why Friday?",
        false,
    ));
    let mut app =
        last_app_with(&[("m3", "Ship the **login page** by Friday.\n")], true, RecordingDelivery::default());
    app.review_in_journal(Ok(Some(journal.clone())));
    assert_eq!(placed_words(&app), ["login page** by"]);
}

#[test]
fn a_note_the_journal_did_not_take_stays_here_marked_until_a_write_takes_it() {
    let (mut app, journal, _) = loop_app();
    journal.state.borrow_mut().refused.push("comment");
    app.add_quote_annotation("We ship", Kind::Comment, "Why Friday?".into()).expect("comment");
    app.poll_journal();
    assert_eq!(placed_words(&app), ["We ship"], "its words are not lost");
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert_eq!(app.journal_tag(&id), " \u{b7} not saved");
    journal.state.borrow_mut().refused.clear();
    assert!(app.open.store.edit_body(&id, "Why not Monday?".into()).expect("edit"));
    app.poll_journal();
    assert_eq!(journal.state.borrow().comments[0].text, "Why not Monday?");
    assert_eq!(app.journal_tag(&id), " \u{b7} draft");
}

#[test]
fn a_notes_kind_and_images_go_with_its_words_and_come_back() {
    let (mut app, journal, _) = loop_app();
    app.add_quote_annotation("We ship on Friday.", Kind::Delete, String::new()).expect("delete");
    app.add_quote_annotation("on-call", Kind::Comment, "Who?".into()).expect("comment");
    app.poll_journal();
    let id = app
        .open
        .store
        .placed()
        .iter()
        .find(|p| p.annotation.body == "Who?")
        .expect("the comment")
        .annotation
        .id
        .clone();
    assert!(app.open.store.add_image(&id, "/tmp/shot one.png").expect("image"));
    app.poll_journal();
    {
        let state = journal.state.borrow();
        let deletion = state.comments.iter().find(|c| c.quote == "We ship on Friday.").expect("the deletion");
        assert!(deletion.deletion && deletion.text.is_empty(), "a delete is the journal's deletion");
        let comment = state.comments.iter().find(|c| c.id == id).expect("the comment");
        assert_eq!(
            comment.text, "Who?\n\nImage: /tmp/shot one.png",
            "an image is named as the feedback names it"
        );
    }
    let note = app.open.store.placed().into_iter().find(|p| p.annotation.id == id).expect("read back");
    assert_eq!(note.annotation.body, "Who?");
    assert_eq!(note.annotation.plannotator_tui.images, ["/tmp/shot one.png"]);
}
