//! The review loop over Plannotator's own CLI, its journal in a folder of its own: two rounds
//! of `last` with the agent answering through `plannotator reply`, the paste into the agent's
//! pane recorded, then the pane closed and opened again. It needs a Plannotator build with the
//! review journal, so it runs only when asked:
//!
//! ```sh
//! PLANNOTATOR_DATA_DIR=<an empty folder> PLANNOTATOR_TUI_TRIAL_BIN=<plannotator> \
//!   cargo test -p plannotator-tui -- --ignored real_cli
//! ```
//!
//! Both this app and the agent's `plannotator` resolve the data directory themselves, so the
//! test runs only with a `PLANNOTATOR_DATA_DIR` of its own, never on the person's.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use plannotator_tui_journal::Cli;
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event as Input, KeyCode};
use serde_json::Value;

use super::super::super::App;
use super::super::super::review_test_support::{RecordingDelivery, draw};
use super::{last_app, press};

fn open(program: &Path, delivery: &RecordingDelivery) -> App {
    let cli = Cli::new(program).probe().expect("a Plannotator with the review journal");
    let mut app = last_app(true, delivery.clone());
    app.review_in_journal(Ok(Some(Rc::new(cli))));
    assert!(app.open.store.is_journal(), "{:?}", app.status);
    app
}

/// What the agent's `plannotator` does, in the same environment.
fn plannotator(program: &Path, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().expect("runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn row_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    let found = screen.lines().find(|row| row.contains(needle));
    assert!(found.is_some(), "{needle}: {screen}");
    found.unwrap_or_default()
}

#[test]
#[ignore = "needs a Plannotator build with the review journal: PLANNOTATOR_TUI_TRIAL_BIN"]
fn real_cli_two_rounds_and_a_pane_opened_again() {
    let program =
        PathBuf::from(std::env::var("PLANNOTATOR_TUI_TRIAL_BIN").expect("PLANNOTATOR_TUI_TRIAL_BIN"));
    let data =
        PathBuf::from(std::env::var("PLANNOTATOR_DATA_DIR").expect("a PLANNOTATOR_DATA_DIR of its own"));
    assert!(std::fs::read_dir(&data).map_or(true, |mut entries| entries.next().is_none()), "an empty one");
    let delivery = RecordingDelivery::default();
    let mut app = open(&program, &delivery);

    // Round 1: a comment and a deletion with an image, sent; the agent's pane gets the text.
    app.add_quote_annotation("turns the new checkout off", Kind::Comment, "Who watches at night?".into())
        .expect("comment");
    app.add_quote_annotation("We ship on Friday.", Kind::Delete, String::new()).expect("deletion");
    app.poll_journal();
    let (comment, deletion) = {
        let placed = app.open.store.placed();
        (placed[0].annotation.id.clone(), placed[1].annotation.id.clone())
    };
    assert!(app.open.store.add_image(&deletion, "/tmp/shot.png").expect("image"));
    app.poll_journal();
    assert!(draw(&mut app, 100, 30).lines().next().unwrap_or_default().contains("round 1 \u{b7} Send 2"));
    press(&mut app, KeyCode::Char('E'));
    let first = delivery.calls.borrow().last().cloned().expect("pasted");
    assert!(first.starts_with("# Comments on your work\n\nThe person finished round 1."), "{first}");
    assert!(
        first.contains(&format!("(id {comment})"))
            && first.contains(&format!(
                "Remove this, on your message m2: \"We ship on Friday.\" (id {deletion})"
            )),
        "{first}"
    );
    assert!(first.contains("> Image: /tmp/shot.png"), "{first}");
    let key = first
        .split("plannotator reply --review ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("the review key")
        .to_owned();
    assert!(!app.quit);

    // The agent answers under both ids through the CLI, another process.
    plannotator(&program, &["reply", "--review", &key, &comment, "A pager rotation; see the runbook."]);
    plannotator(&program, &["reply", "--review", &key, &deletion, "Removed."]);
    assert!(app.poll_journal(), "the journal moved");
    let screen = draw(&mut app, 80, 40);
    assert!(row_with(&screen, "\"turns the new checkout").contains("replied"), "{screen}");
    assert!(row_with(&screen, "Claude Code").contains("new"), "{screen}");

    // Round 2: the person's reply, a sub-thread on the agent's words, a resolution.
    app.select_block(1);
    press(&mut app, KeyCode::Char('C'));
    app.handle_event(&Input::Paste("And on weekends?".into())).expect("typed");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('O'));
    app.add_quote_annotation("the runbook", Kind::Comment, "Which runbook?".into()).expect("sub-thread");
    press(&mut app, KeyCode::Esc);
    app.poll_journal();
    app.select_block(2);
    press(&mut app, KeyCode::Char('V'));
    press(&mut app, KeyCode::Char('E'));
    let second = delivery.calls.borrow().last().cloned().expect("pasted");
    assert!(second.contains("The person finished round 2."), "{second}");
    assert!(second.contains(&format!("Reply under comment {comment}")), "{second}");
    assert!(
        second.contains("Comment on a passage of reply rep_") && second.contains("Which runbook?"),
        "{second}"
    );

    // The agent's view: delivered, read and replied as the journal says.
    let view: Value =
        serde_json::from_str(&plannotator(&program, &["threads", "--review", &key, "--json"])).expect("JSON");
    let threads = view["threads"].as_array().expect("threads");
    let state =
        |id: &str| threads.iter().find(|t| t["id"] == id).map(|t| (t["state"].clone(), t["status"].clone()));
    assert_eq!(state(&comment), Some((Value::from("open"), Value::from("replied"))), "{view}");
    let mine =
        threads[0]["replies"].as_array().expect("replies").iter().find(|r| r["author"]["kind"] == "person");
    assert_eq!(
        mine.map(|reply| reply["status"].clone()),
        Some(Value::from("delivered")),
        "round 2 reached the agent"
    );
    assert_eq!(state(&deletion).map(|(state, _)| state), Some(Value::from("resolved")), "{view}");

    // The pane closed and opened again: the review is the session's, kept by the journal.
    let mut again = open(&program, &delivery);
    let screen = draw(&mut again, 80, 40);
    assert!(screen.contains("A pager rotation; see the runbook."), "{screen}");
    assert!(row_with(&screen, "round 3").contains("Sent"), "{screen}");
    let notes = again.open.store.placed();
    let image = notes.iter().find(|p| p.annotation.id == deletion).expect("the deletion");
    assert_eq!(image.annotation.plannotator_tui.images, ["/tmp/shot.png"]);
}
