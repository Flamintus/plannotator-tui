//! The review loop over a file or a folder: its notes are the journal's comments on each
//! file, `E` ends a round sent to the agent's session, a gated review ends with Approve,
//! which closes the review, and the Review menu leaves the journal's comments alone.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::rc::Rc;

use plannotator_tui_journal::{Anchor as On, ReviewTarget, SendMode, SendTo, Status};
use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::crossterm::event::KeyCode;

use super::super::super::review_test_support::{RecordingDelivery, draw, file_app, folder_app};
use super::super::super::{App, Open};
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
            .contains("Round 1 \u{b7} Send Feedback (2) \u{25b8} test agent")
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
fn the_review_menu_acts_on_the_journals_threads() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = file_loop("menu", &journal);
    app.add_quote_annotation("one", Kind::Comment, "First.".into()).expect("comment");
    let id = app.open.store.placed()[0].annotation.id.clone();
    press(&mut app, KeyCode::Char('E'));
    app.add_quote_annotation("two", Kind::Comment, "A draft.".into()).expect("draft");
    press(&mut app, KeyCode::Char('F'));
    assert_eq!(writes(&journal).last().map(String::as_str), Some(format!("resolve {id} true").as_str()));
    assert_eq!(app.status.as_deref(), Some("resolved 1 sent thread(s)"), "the draft stays");
    press(&mut app, KeyCode::Char('U'));
    assert_eq!(writes(&journal).last().map(String::as_str), Some(format!("resolve {id} false").as_str()));
    press(&mut app, KeyCode::Char('F'));
    press(&mut app, KeyCode::Char('H'));
    assert_eq!(app.archive_items.len(), 1, "the archive lists the resolved thread");
    press(&mut app, KeyCode::Enter);
    assert_eq!(writes(&journal).last().map(String::as_str), Some(format!("resolve {id} false").as_str()));
    press(&mut app, KeyCode::Char('R'));
    let sends = journal.state.borrow().sends.clone();
    assert_eq!(sends.len(), 2, "resend all: a round naming the threads still open");
    assert_eq!(delivery.calls.borrow().len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_files_own_notes_become_drafts_of_its_review_once_and_stay_in_its_record() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, _) = file_app("own");
    app.add_quote_annotation("two", Kind::Comment, "Kept outside the loop.".into()).expect("note");
    let id = app.open.store.placed()[0].annotation.id.clone();
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    assert_eq!(writes(&journal), [format!("comment {id}")]);
    assert!(app.status.as_deref().unwrap_or_default().contains("1 note(s) kept outside the loop"));
    journal.state.borrow_mut().comments.clear();
    // The pane opened again on an empty review (one an approval closed): nothing goes twice.
    let path = file_of(&app);
    app.open = Open::new(crate::app::read_file(&path).expect("read"), 100, &app.data_dir, &app.project)
        .expect("open");
    app.review_loop = None;
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    assert_eq!(writes(&journal).len(), 1);
    let location = crate::store::Location::for_file(&app.data_dir, &app.project, &path);
    let own = crate::store::Store::load(&location, &app.open.doc).expect("its record");
    assert_eq!(own.len(), 1, "the record keeps it for when the loop is off");
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
fn a_send_an_earlier_pane_left_pending_goes_again_before_the_drafts_a_screens_does_not() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = file_app("left");
    let path = file_of(&app).display().to_string();
    {
        // A Plannotator screen sent c1, an earlier pane c2; neither reached the agent.
        let mut state = journal.state.borrow_mut();
        for (n, (id, key)) in [("c1", "press-1"), ("c2", "tui_1_2_3")].into_iter().enumerate() {
            let on = On::File { path: path.clone(), lines: Some([3, 3]) };
            let mut sent = Comment::draft(id, on, "one", "Words.");
            (sent.draft, sent.status) = (false, Some(Status::Pending));
            sent.send = Some(format!("snd_{}", n + 1));
            state.comments.push(sent);
            state.sends.push((SendMode::Now, vec![id.into()], key.into()));
            state.sent_to.push(Some(to()));
        }
    }
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    app.add_quote_annotation("two", Kind::Comment, "Second.".into()).expect("draft");
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(journal.state.borrow().resent, ["snd_1", "snd_2"], "the screen's is left to its host");
    assert_eq!(delivery.calls.borrow().as_slice(), ["# Comments on your work\n\nround 1: c2"]);
    assert_eq!(journal.state.borrow().sends.len(), 2, "the draft waits for the next press");
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(journal.state.borrow().sends.len(), 3);
    assert_eq!(delivery.calls.borrow().len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_approval_carries_every_draft_even_on_a_file_the_agent_removed() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = folder_app("approve-all");
    app.gate = Gate::Approve;
    let gone = root.join("docs/gone.md");
    std::fs::write(&gone, "# Gone\n\nOld words.\n").expect("gone.md");
    let gone_path = real_path(&gone).display().to_string();
    let here = file_of(&app).display().to_string();
    {
        let mut state = journal.state.borrow_mut();
        let mut sent = Comment::draft("c1", On::File { path: here, lines: Some([3, 3]) }, "one", "Sent.");
        (sent.draft, sent.send) = (false, Some("snd_0".into()));
        state.comments.push(sent.clone());
        let mut reply = Comment::draft("r1", sent.on.clone(), "", "A reply of mine.");
        reply.re = Some("c1".into());
        state.comments.push(reply);
        let mut sub = Comment::draft("s1", sent.on.clone(), "Sent", "On the passage.");
        sub.parent = Some("c1".into());
        state.comments.push(sub);
        let on_gone = On::File { path: gone_path, lines: Some([3, 3]) };
        state.comments.push(Comment::draft("g1", on_gone, "Old words.", "Drop this file."));
    }
    std::fs::remove_file(&gone).expect("the agent removed it");
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    press(&mut app, KeyCode::Char('A'));
    let approval = delivery.calls.borrow().last().cloned().expect("the approval");
    for words in ["A reply of mine.", "On the passage.", "Drop this file.", "# Annotations on gone.md"] {
        assert!(approval.contains(words), "{words}: {approval}");
    }
    assert!(!approval.contains("(id "), "the review closes: no id to answer under");
    assert!(app.quit && journal.state.borrow().closed);
    assert!(matches!(app.take_decision(), Some(Decision::Approved { count: 3, .. })));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_approval_the_journal_does_not_close_keeps_the_window_and_says_so() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = file_app("close-refused");
    app.gate = Gate::Approve;
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    journal.state.borrow_mut().refused.push("close");
    press(&mut app, KeyCode::Char('A'));
    assert_eq!(delivery.calls.borrow().as_slice(), ["The user approved."]);
    assert!(!app.quit, "the footer can say it");
    assert!(app.status.as_deref().unwrap_or_default().contains("the review stays in the journal"));
    assert!(matches!(app.take_decision(), Some(Decision::Approved { .. })), "the decision stands");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_folder_opened_on_no_file_of_its_own_sends_its_round_and_counts_it_sent() {
    let journal = Rc::new(StandIn::default());
    let (root, mut app, delivery) = folder_app("placeholder");
    let on = On::File { path: file_of(&app).display().to_string(), lines: Some([3, 3]) };
    journal.state.borrow_mut().comments.push(Comment::draft("d1", on, "one", "Left before."));
    let source = DocumentSource::new("# docs\n\nNo Markdown file.\n".into(), "docs", true, Provenance::Stdin);
    app.open = Open::new(source, 100, &app.data_dir, &app.project).expect("placeholder");
    app.gate = Gate::Approve;
    app.review_path_in_journal(Ok(Some((journal.clone(), to()))));
    press(&mut app, KeyCode::Char('E'));
    assert_eq!(delivery.calls.borrow().as_slice(), ["# Comments on your work\n\nround 1: d1"]);
    assert_eq!(app.send_count(), 0, "the view was read again");
    press(&mut app, KeyCode::Char('A'));
    assert_eq!(
        delivery.calls.borrow().last().map(String::as_str),
        Some("The user approved."),
        "nothing twice"
    );
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
