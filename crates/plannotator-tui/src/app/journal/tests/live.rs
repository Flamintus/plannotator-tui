//! The thread followed on the Inbox's event stream.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::rc::Rc;

use plannotator_tui_journal::{Error, Event, Kind, Record};
use ratatui::crossterm::event::KeyCode;

use super::super::super::App;
use super::{press, thread_app_of, thread_inbox};
use crate::journal::stand_in::{Comment, StandIn};

/// `thread_inbox`'s newest message open, its thread followed since the first wake.
fn followed() -> (App, Rc<StandIn>) {
    let inbox = thread_inbox();
    let mut app = thread_app_of(&inbox);
    press(&mut app, KeyCode::Enter);
    app.poll_inbox();
    (app, inbox)
}

fn tell(inbox: &StandIn, event: Event) {
    let state = inbox.state.borrow();
    state.stream.as_ref().expect("a watch").send(event).expect("the app listens");
}

fn record(id: &str) -> Record {
    Record { seq: 50, kind: Kind::Annotation, id: id.into(), thread_id: Some("thr_1".into()) }
}

fn comment_from_the_window(inbox: &StandIn) {
    let comment = Comment::on_message("c9", "msg_a2", "Second", "From the window.", false);
    inbox.state.borrow_mut().comments.push(comment);
}

#[test]
fn the_open_documents_thread_is_followed_from_the_first_wake() {
    let (mut app, inbox) = followed();
    assert_eq!(inbox.state.borrow().watched, [Some("thr_1".to_owned())]);
    app.poll_inbox();
    assert_eq!(inbox.state.borrow().watched.len(), 1, "one stream per thread");
}

#[test]
fn a_record_of_the_thread_has_its_view_read_again() {
    let (mut app, inbox) = followed();
    comment_from_the_window(&inbox);
    assert!(!app.poll_inbox(), "nothing told, nothing read");
    tell(&inbox, Event::Records(vec![record("c9")]));
    assert!(app.poll_inbox());
    let bodies: Vec<String> = app.open.store.placed().iter().map(|p| p.annotation.body.clone()).collect();
    assert_eq!(bodies, ["From the window."]);
}

#[test]
fn a_restarted_inbox_is_read_again_and_said() {
    let (mut app, inbox) = followed();
    comment_from_the_window(&inbox);
    tell(&inbox, Event::Restarted { server_session: "ss_two".into(), cursor: 3 });
    assert!(app.poll_inbox());
    assert_eq!(app.open.store.placed().len(), 1);
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("restarted"), "{status}");
}

#[test]
fn a_lost_connection_is_said_once_and_so_is_its_return() {
    let (mut app, inbox) = followed();
    let lost = || Event::Disconnected(Error::Transport { route: "events".into(), detail: "reset".into() });
    tell(&inbox, lost());
    assert!(app.poll_inbox());
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("connection lost"), "{status}");
    app.status = None;
    tell(&inbox, lost());
    app.poll_inbox();
    assert_eq!(app.status, None, "said once while it lasts");
    tell(&inbox, Event::Open { cursor: 60 });
    app.poll_inbox();
    assert_eq!(app.status.as_deref(), Some("Inbox: connected again"));
}

#[test]
fn a_stopped_stream_is_said_and_not_started_again() {
    let (mut app, inbox) = followed();
    tell(&inbox, Event::Stopped(Error::Unpaired { code: "device_revoked".into() }));
    app.poll_inbox();
    let status = app.status.clone().unwrap_or_default();
    assert!(status.contains("no longer following"), "{status}");
    app.poll_inbox();
    assert_eq!(inbox.state.borrow().watched.len(), 1);
}
