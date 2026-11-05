//! Following the Inbox's event stream, `GET events` through the door, on threads of its own.
//!
//! The stream (`packages/server/inbox.ts`, `eventStream`): `retry: 2000`, an `event: hello`
//! `{ serverSession, cursor }`, then one `event: record` `{ seq, kind, id, … }` per store
//! line with `id: <seq>`, and a `: ping` every 15 s. A reader resumes by `Last-Event-ID`.
//! E7 adds `v` and `thread_id` to every line; until then a message's and a comment's thread
//! is read from the record they carry.
//!
//! A watch never replays records into state: it says what changed, a few lines at a time,
//! and the caller reads the computed view again. A reader thread follows the stream and
//! reconnects (`reader.rs`); a second gathers the records that arrive together into one
//! event.

mod reader;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::http::Door;
use reader::Reader;

/// Records that arrive within this window of the first one are told together.
pub const GATHER: Duration = Duration::from_millis(100);

/// What a watch tells.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The stream is open (first time, or again after a cut, the same Inbox): `cursor` is
    /// the Inbox's last seq now.
    Open { cursor: u64 },
    /// The Inbox restarted (another `serverSession`): read everything again.
    Restarted { server_session: String, cursor: u64 },
    /// Records of the watched thread (all threads' when none is watched), oldest first.
    Records(Vec<Record>),
    /// The connection was lost; the watch connects again after the stream's `retry` delay.
    Disconnected(Error),
    /// The watch ended and does not retry: the token is refused, the door has no event
    /// stream for this client, or the stream is in a newer contract.
    Stopped(Error),
}

/// One store line, as far as a watch tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub seq: u64,
    pub kind: Kind,
    pub id: String,
    pub thread_id: Option<String>,
}

/// The kinds of store lines a watch tells; a kind it does not know is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Project,
    Message,
    Question,
    Annotation,
    Card,
    Steps,
    CommentReply,
    CommentSend,
}

impl Kind {
    pub(crate) fn from_wire(kind: &str) -> Option<Self> {
        Some(match kind {
            "project" => Self::Project,
            "message" => Self::Message,
            "question" => Self::Question,
            "annotation" => Self::Annotation,
            "card" => Self::Card,
            "steps" => Self::Steps,
            "comment_reply" => Self::CommentReply,
            "comment_send" => Self::CommentSend,
            _ => return None,
        })
    }
}

/// A running watch. Dropping it ends its threads: at once while it waits to reconnect, at
/// the stream's next line (a ping at most 15 s away) while it reads.
#[derive(Debug)]
pub struct Watch {
    events: Receiver<Event>,
    /// Never sent on: dropped with the watch, which wakes and ends the reader.
    _stop: Option<Sender<()>>,
}

impl Watch {
    /// A watch fed by `events`: what a stand-in for the Inbox hands the application.
    pub fn from_events(events: Receiver<Event>) -> Self {
        Self { events, _stop: None }
    }

    /// The next event, if one is waiting.
    pub fn try_next(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting up to `timeout`.
    pub fn next_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }

    pub(crate) fn start(door: Door, from: Option<u64>, thread_id: Option<String>) -> Result<Self, Error> {
        let (raw_tx, raw_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();
        let reader = Reader::new(door, from, thread_id, raw_tx, stop_rx);
        let spawn = |name: &str, run: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(run)
                .map_err(|e| Error::transport("events", format!("starting the watch: {e}")))
        };
        spawn("inbox-gather", Box::new(move || gather(&raw_rx, &out_tx)))?;
        spawn("inbox-watch", Box::new(move || reader.run()))?;
        Ok(Self { events: out_rx, _stop: Some(stop_tx) })
    }
}

/// Pass events on, the records that arrive within [`GATHER`] of each other as one.
fn gather(raw: &Receiver<Event>, out: &Sender<Event>) {
    while let Ok(event) = raw.recv() {
        let Event::Records(mut batch) = event else {
            if out.send(event).is_err() {
                return;
            }
            continue;
        };
        let deadline = Instant::now() + GATHER;
        let mut next = None;
        while next.is_none() {
            match raw.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Event::Records(more)) => batch.extend(more),
                Ok(other) => next = Some(other),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        if out.send(Event::Records(batch)).is_err() || next.is_some_and(|event| out.send(event).is_err()) {
            return;
        }
    }
}
