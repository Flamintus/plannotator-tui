//! The flow drawn from the thread view Plannotator's own CLI computed (the client crate's
//! fixture, `view.json`, written by `threads view`): what folds, statuses, drafts and
//! sub-threads are Plannotator's, read as they are.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::Path;
use std::rc::Rc;

use super::super::super::review_test_support::{RecordingDelivery, draw};
use super::{NEWEST, last_app_with};
use crate::journal::stand_in::StandIn;

fn fixture_app() -> crate::app::App {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../plannotator-tui-journal/tests/fixtures/v1/view.json");
    let text = std::fs::read_to_string(&path).expect("the fixture");
    let viewed: plannotator_tui_journal::Viewed = serde_json::from_str(&text).expect("a view");
    let journal = Rc::new(StandIn::default());
    journal.state.borrow_mut().view = Some(viewed.view);
    // The fixture's comments are on reply m1 (these words) and m2.
    let mut app =
        last_app_with(&[("m1", NEWEST), ("m2", "Second reply.\n")], true, RecordingDelivery::default());
    app.review_in_journal(Ok(Some(journal.clone())));
    app
}

#[test]
fn the_view_plannotator_computes_is_drawn_as_it_says() {
    let mut app = fixture_app();
    // Under 80 columns the rail is gone: what is left is the document's own column.
    let screen = draw(&mut app, 70, 60);
    let rows: Vec<&str> = screen.lines().collect();
    let row = |needle: &str| rows.iter().copied().find(|row| row.contains(needle)).expect(needle);
    assert!(
        rows[0].contains("Round 2 \u{b7} Send Feedback (3) \u{25b8} test agent"),
        "the round under way, three drafts: {screen}"
    );
    assert!(row("\"turns the new checkout off\"").contains("Replied"), "{screen}");
    assert!(screen.contains("The on-call engineer, from the runbook."), "the agent's reply: {screen}");
    assert!(screen.contains("You \u{b7} Not sent"), "the person's reply waits: {screen}");
    assert!(
        row("Which runbook?").starts_with("  \u{2502} \u{2502} "),
        "a sub-thread on the agent's reply: {screen}"
    );
    assert!(row("We ship on Friday.\"").contains("Resolved"), "folded, as the view says: {screen}");
    assert!(!screen.contains("Shorter."), "a comment on the other reply is not on this one: {screen}");
}
