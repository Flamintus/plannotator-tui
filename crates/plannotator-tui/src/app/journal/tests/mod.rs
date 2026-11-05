//! The review loop against a stand-in journal (`crate::journal::stand_in`): `last` with the
//! loop off or on (`loop_on`), the comments the journal keeps (`kept`), sending them
//! (`send`), the journal followed and the threads drawn and acted on (`threads`), from a view
//! Plannotator's own CLI computed too (`contract`); sub-threads on a passage selected in a
//! box (`words`).

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

mod contract;
mod kept;
mod loop_on;
mod paths;
mod real;
mod send;
mod threads;
mod words;

use std::rc::Rc;

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use super::super::App;
use super::super::review_test_support::RecordingDelivery;
use crate::journal::stand_in::StandIn;

/// The newest reply `last` offers, with a heading and two points.
pub(super) const NEWEST: &str =
    "# Rollout\n\nThe on-call engineer turns the new checkout off.\n\nWe ship on Friday.\n";

/// `last` on two replies of Claude Code's session `ses_a`, newest first, without the loop.
pub(super) fn last_app(newest: bool, delivery: RecordingDelivery) -> App {
    last_app_with(&[("m2", NEWEST), ("m1", "First reply.\n")], newest, delivery)
}

/// `last` on `replies`, `(id, words)` newest first, of Claude Code's session `ses_a`.
pub(super) fn last_app_with(replies: &[(&str, &str)], newest: bool, delivery: RecordingDelivery) -> App {
    let replies = replies
        .iter()
        .map(|(id, text)| plannotator_tui_hosts::Message {
            id: (*id).into(),
            role: plannotator_tui_hosts::Role::Assistant,
            text: (*text).into(),
            at: None,
        })
        .collect();
    App::open_message(
        "claude",
        "/tmp/transcript.jsonl",
        Some("ses_a"),
        replies,
        80,
        Box::new(delivery),
        newest,
    )
    .expect("app")
}

/// The same with the review loop on, over `journal`.
pub(super) fn loop_app_of(journal: &Rc<StandIn>, delivery: RecordingDelivery) -> App {
    let mut app = last_app(true, delivery);
    app.review_in_journal(Ok(Some(journal.clone())));
    app
}

/// A review with the loop on over an empty journal, the newest reply open.
pub(super) fn loop_app() -> (App, Rc<StandIn>, RecordingDelivery) {
    let journal = Rc::new(StandIn::default());
    let delivery = RecordingDelivery::default();
    (loop_app_of(&journal, delivery.clone()), journal, delivery)
}

pub(super) fn press(app: &mut App, code: KeyCode) {
    app.handle_event(&Event::Key(KeyEvent::from(code))).expect("key");
}

pub(super) fn writes(journal: &StandIn) -> Vec<String> {
    journal.state.borrow().writes.clone()
}
