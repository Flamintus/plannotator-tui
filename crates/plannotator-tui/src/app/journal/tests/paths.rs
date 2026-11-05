//! The review loop over a file or a folder: its notes are the journal's comments on each
//! file, `E` ends a round sent to the agent's session, a gated review ends with Approve,
//! which closes the review, and the Review menu leaves the journal's comments alone.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::rc::Rc;

use plannotator_tui_journal::{Anchor as On, ReviewTarget, SendMode, SendTo, Status};
use plannotator_tui_schema::{Kind, Provenance};
use ratatui::crossterm::event::KeyCode;

use super::super::super::App;
use super::super::super::review_test_support::{RecordingDelivery, draw, file_app, folder_app};
use super::{press, writes};
use crate::gate::{Decision, Gate};
use crate::journal::real_path;
use crate::journal::stand_in::{Comment, StandIn};

/// The agent session the rounds go to.
fn to() -> SendTo {
    SendTo { host: Some("claude-code".into()), session: "ses_f".into() }
}

fn file_of(app: &App) -> PathBuf {
    let Provenance::File { path } = &app.open.source.provenance else { unreachable!("a file review") };
    real_path(path)
}

/// A file review, with the loop on over `journal`.
fn file_loop(tag: &str, journal: &Rc<StandIn>) -> (PathBuf, App, RecordingDelivery) {
    let (root, mut app, delivery) = file_app(tag);
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    (root, app, delivery)
}

#[test]
fn a_files_notes_are_its_reviews_drafts_and_e_ends_a_round_sent_to_the_agents_session() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = file_loop("round", &journal);
    let file = file_of(&app);
    app.add_quote_annotation("two", Kind::Comment, "Say more.".into()).expect("comment");
    app.add_quote_annotation("three", Kind::Delete, String::new()).expect("deletion");
    app.poll_journal();
    {
        let state = journal.state.borrow();
        assert_eq!(state.targets[0], ReviewTarget::File { path: file.display().to_string() });
        let path = file.display().to_string();
        assert_eq!(state.comments[0].on, On::File { path: path.clone(), lines: Some([5, 5]) });
        assert_eq!(
            (state.comments[1].on.clone(), state.comments[1].deletion),
            (On::File { path, lines: Some([7, 7]) }, true)
        );
    }
    let kinds: Vec<Kind> = app.open.store.placed().iter().map(crate::store::Placed::kind).collect();
    assert_eq!(kinds, [Kind::Comment, Kind::Delete], "a deletion read back is one");
    assert!(
        draw(&mut app, 100, 12)
            .lines()
            .next()
            .unwrap_or_default()
            .contains("round 1 \u{b7} Send 2 to test agent")
    );
    press(&mut app, KeyCode::Char('E'));
    {
        let state = journal.state.borrow();
        assert_eq!(
            state.sends.iter().map(|(mode, ids, _)| (*mode, ids.len())).collect::<Vec<_>>(),
            [(SendMode::Round, 2)]
        );
        assert_eq!(state.sent_to, [Some(to())], "a file's round names the agent's session");
        assert_eq!(state.delivered, ["snd_1"]);
    }
    assert_eq!(delivery.calls.borrow().len(), 1);
    assert!(delivery.calls.borrow()[0].starts_with("# Comments on your work"));
    assert!(!app.quit);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_gated_review_ends_rounds_with_e_and_the_review_with_approve() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = file_app("gate");
    app.gate = Gate::Approve;
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    app.add_quote_annotation("one", Kind::Comment, "First.".into()).expect("comment");
    press(&mut app, KeyCode::Char('E'));
    assert!(!app.quit, "a round is not the decision");
    assert_eq!(app.decision, None);
    app.add_quote_annotation("two", Kind::Comment, "Keep it short.".into()).expect("draft");
    app.poll_journal();
    assert!(draw(&mut app, 120, 12).lines().next().unwrap_or_default().contains("Approve with 1 note (A)"));
    press(&mut app, KeyCode::Char('A'));
    let approval = delivery.calls.borrow().last().cloned().expect("the approval");
    assert!(approval.starts_with("# Approved with Notes"), "{approval}");
    assert!(approval.contains("Keep it short."), "the draft rides along: {approval}");
    assert!(!approval.contains("First."), "a sent comment does not: {approval}");
    assert!(app.quit);
    assert!(matches!(app.take_decision(), Some(Decision::Approved { count: 1, .. })));
    assert!(journal.state.borrow().closed, "the approval closes the review");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_review_menu_leaves_the_journals_comments_alone() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = file_loop("menu", &journal);
    app.add_quote_annotation("one", Kind::Comment, "First.".into()).expect("comment");
    press(&mut app, KeyCode::Char('E'));
    let before = writes(&journal);
    for key in ['R', 'F', 'U', 'H'] {
        press(&mut app, KeyCode::Char(key));
        assert!(app.status.as_deref().unwrap_or_default().contains("outside the loop"), "{key}");
    }
    assert_eq!(writes(&journal), before);
    assert_eq!(delivery.calls.borrow().len(), 1, "nothing resent");
    assert_eq!(app.open.store.placed().len(), 1, "nothing archived");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_folders_tree_counts_the_reviews_comments_by_file() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, _) = folder_app("folder");
    let other = root.join("docs/b.md");
    std::fs::write(&other, "# B\n\nbee\n").expect("b.md");
    app.tree = Some(crate::tree::Tree::scan(&root.join("docs")).expect("tree"));
    let on = On::File { path: real_path(&other).display().to_string(), lines: None };
    journal.state.borrow_mut().comments.push(Comment::draft("b1", on, "bee", "Left by an earlier pane."));
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("1 draft(s) left by an earlier pane go with the next E"), "{status}");
    assert_eq!(
        journal.state.borrow().targets[0],
        ReviewTarget::File { path: real_path(&root.join("docs")).display().to_string() },
        "one review for the folder"
    );
    let count = |app: &App, name: &str| {
        let tree = app.tree.as_ref().expect("tree");
        tree.rows.iter().find(|row| row.path.ends_with(name)).map(|row| row.annotations)
    };
    assert_eq!(count(&app, "b.md"), Some(1));
    app.add_quote_annotation("two", Kind::Comment, "Say more.".into()).expect("comment");
    app.poll_journal();
    assert_eq!((count(&app, "a.md"), count(&app, "b.md")), (Some(1), Some(1)));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_send_an_earlier_pane_left_pending_goes_again_before_the_drafts() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = file_app("left");
    let path = file_of(&app).display().to_string();
    {
        // An earlier pane sent c1; the agent never got it.
        let mut state = journal.state.borrow_mut();
        let mut sent = Comment::draft("c1", On::File { path, lines: Some([3, 3]) }, "one", "First.");
        (sent.draft, sent.status, sent.send) = (false, Some(Status::Pending), Some("snd_1".into()));
        state.comments.push(sent);
        state.sends.push((SendMode::Round, vec!["c1".into()], "gone".into()));
        state.sent_to.push(Some(to()));
        state.finished = 1;
    }
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    app.add_quote_annotation("two", Kind::Comment, "Second.".into()).expect("draft");
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(journal.state.borrow().resent, ["snd_1"]);
    assert_eq!(delivery.calls.borrow().as_slice(), ["# Comments on your work\n\nround 1: c1"]);
    assert_eq!(journal.state.borrow().sends.len(), 1, "the draft waits for the next press");
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(journal.state.borrow().sends.len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_review_that_ends_with_its_first_send_stays_out_of_the_loop() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, _) = file_app("decision");
    app.gate = Gate::Decision;
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    assert!(!app.review_loop_runs());
    assert!(app.status.as_deref().unwrap_or_default().starts_with("review loop off"));
    assert!(journal.state.borrow().targets.is_empty(), "the journal is never called");
    let _ = std::fs::remove_dir_all(root);
}
