//! The flow drawn from the thread view Plannotator's own code computes (the client crate's
//! contract fixture, `thread-view.json`, made by E1's `inboxCommentThreadView`): what folds,
//! statuses, live threads, drafts and sub-threads are the server's, read as they are.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::Path;
use std::rc::Rc;

use super::super::super::review_test_support::draw;
use super::thread_app_of;
use crate::inbox::stand_in::{StandIn, thread};

/// The fixture's comments sit on message `msg_a1`, on these words.
const BODY: &str = "words of c0\n\nwords of c1 comment\n\nwords of c9\n\nwords of d2\n";

fn fixture_app() -> crate::app::App {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../plannotator-tui-inbox/tests/fixtures/v1/thread-view.json");
    let text = std::fs::read_to_string(&path).expect("the contract fixture");
    let inbox = Rc::new(StandIn::default());
    {
        let mut state = inbox.state.borrow_mut();
        state.threads.push(thread("thr_1", "The plan", &[("msg_a1", true, BODY, serde_json::json!([]))]));
        state.view = Some(serde_json::from_str(&text).expect("a view"));
    }
    let mut app = thread_app_of(&inbox);
    app.poll_inbox();
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
        rows[0].contains("round 3 \u{b7} Send 2 now \u{25b8} Inbox"),
        "the round under way, two drafts: {screen}"
    );
    assert!(row("\"words of c1\"").contains("replied \u{b7} live"), "{screen}");
    assert!(screen.contains("answer rep_a"), "the agent's reply: {screen}");
    assert!(screen.contains("You \u{b7} pending") && screen.contains("You \u{b7} draft"), "{screen}");
    assert!(
        row("deeper").starts_with("  \u{2502} \u{2502} \u{2502} "),
        "a sub-thread in a sub-thread: {screen}"
    );
    assert!(row("\"words of c9\"").contains("\u{2713}"), "folded, as the view says: {screen}");
    assert!(!screen.contains("comment d2"), "a draft no one answered stays a bubble: {screen}");
    assert!(!screen.contains("fn main()"), "comments on files and diffs are not on this message: {screen}");
}
