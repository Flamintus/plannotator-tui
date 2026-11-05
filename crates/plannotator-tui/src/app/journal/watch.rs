//! Following the Inbox: the open document's thread is watched on the Inbox's event stream
//! (`Inbox::watch`, on threads of its own), and what it told is read on every wake of the
//! event loop. Records of the thread (a reply, a send, another client's comment) have its
//! view read again; so does the stream opening, since it starts after the first read. A
//! restarted Inbox is read again and said; a lost connection is said once, and its return,
//! while the stream picks up where it left off. The view is always read again, never
//! rebuilt from the records.

use plannotator_tui_journal::{Event, Watch};

use super::super::App;

/// The thread followed, and its stream while it runs.
pub(super) struct Following {
    thread_id: String,
    /// `None` once the stream stopped for good, or could not start.
    watch: Option<Watch>,
    /// The connection is lost; the stream connects again on its own.
    lost: bool,
}

/// What the stream told since the last wake.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Told {
    /// The thread may have changed: read its view again.
    pub(super) changed: bool,
    /// The footer has something new to say.
    pub(super) said: bool,
}

impl App {
    /// Follow the open document's thread, unless it is followed already.
    fn follow(&mut self) {
        let Some((inbox, thread_id, _)) = self.open.store.inbox_thread() else { return };
        let Some(review) = &self.inbox else { return };
        if review.following.as_ref().is_some_and(|following| following.thread_id == thread_id) {
            return;
        }
        let (thread_id, started) = (thread_id.to_owned(), inbox.watch(None, Some(thread_id)));
        let watch = match started {
            Ok(watch) => Some(watch),
            Err(err) => {
                self.status = Some(format!("Inbox: not following the thread: {err}"));
                None
            }
        };
        if let Some(review) = self.inbox.as_mut() {
            review.following = Some(Following { thread_id, watch, lost: false });
        }
    }

    /// Follow the open thread and read what its stream told since the last wake.
    pub(super) fn read_stream(&mut self) -> Told {
        self.follow();
        let mut told = Told::default();
        let mut status = None;
        let Some(following) = self.inbox.as_mut().and_then(|review| review.following.as_mut()) else {
            return told;
        };
        while let Some(event) = following.watch.as_ref().and_then(Watch::try_next) {
            match event {
                Event::Open { .. } => {
                    told.changed = true;
                    if std::mem::take(&mut following.lost) {
                        status = Some("Inbox: connected again".to_owned());
                    }
                }
                Event::Restarted { .. } => {
                    told.changed = true;
                    status = Some("the Inbox restarted: its comments are read again".to_owned());
                }
                Event::Records(records) => told.changed |= !records.is_empty(),
                Event::Disconnected(err) => {
                    if !std::mem::replace(&mut following.lost, true) {
                        status = Some(format!("Inbox: connection lost ({err}) \u{b7} connecting again"));
                    }
                }
                Event::Stopped(err) => {
                    following.watch = None;
                    status = Some(format!("Inbox: no longer following the thread: {err}"));
                }
            }
        }
        if let Some(status) = status {
            self.status = Some(status);
            told.said = true;
        }
        told
    }
}
