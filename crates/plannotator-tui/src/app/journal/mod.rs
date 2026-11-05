//! The review loop in the app (`crate::journal`): `last`'s replies opened as documents whose
//! comments Plannotator's review journal keeps, by host and session (here), or the files of a
//! file's or a folder's review, by its real path (`paths`); the round sent and pasted into the
//! agent's pane as the journal composed it (`send`); the journal followed for the agent's
//! replies (`watch`); and replies, resolution and sub-threads on the current note's thread
//! (`act`).

mod act;
mod left;
mod paths;
mod send;
mod watch;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::Result;
use plannotator_tui_journal::{ReviewTarget, Stamp, ThreadNode, ThreadView};
use plannotator_tui_schema::{DocumentSource, Provenance};

use super::{App, Mode, Open};
use crate::journal::{Review, Shared, Target, real_path, tree};
use crate::store::Store;

/// What the review loop keeps besides its documents.
pub(super) struct ReviewLoop {
    review: Review,
    /// Why the journal could not list the comments of the reply opened last.
    unlisted: Option<String>,
    /// The last send the agent did not get, to hand over again.
    press: Option<send::Press>,
    /// The sends nothing marked delivered when the loop started, by id, the oldest last: an
    /// earlier pane's that the agent may not have got.
    left: Vec<String>,
    /// The review's journal as last read: a new stamp means another process wrote it.
    stamp: Option<Stamp>,
    /// The agent's replies the person has seen: the ones there when the review first read
    /// the journal, and those under a thread the person acted on since. Others are new.
    seen: HashSet<String>,
    seeded: bool,
    /// The documents left for comments' threads opened as documents, the last one last.
    parents: Vec<act::Parked>,
    /// When the view could not be read last: it is tried again after `RETRY`, not on every
    /// wake, since each try runs the CLI.
    failed: Option<Instant>,
    /// A folder's review opened on no file of its own: its view, as read when it opened.
    view: Option<ThreadView>,
}

/// How long a view that could not be read waits before it is read again.
const RETRY: Duration = Duration::from_secs(2);

impl ReviewLoop {
    fn new(review: Review) -> Self {
        Self {
            review,
            unlisted: None,
            press: None,
            left: Vec::new(),
            stamp: None,
            seen: HashSet::new(),
            seeded: false,
            parents: Vec::new(),
            failed: None,
            view: None,
        }
    }

    /// The agent's replies seen; the first call takes every reply there as seen.
    pub(in crate::app) fn seen(&mut self, view: &ThreadView) -> &HashSet<String> {
        if !std::mem::replace(&mut self.seeded, true) {
            let mut replies = Vec::new();
            for node in &view.threads {
                tree::agent_replies(node, &mut replies);
            }
            self.seen.extend(replies);
        }
        &self.seen
    }

    /// The person acted on `node`: its replies are seen.
    fn saw(&mut self, node: &ThreadNode) {
        let mut replies = Vec::new();
        tree::agent_replies(node, &mut replies);
        self.seen.extend(replies);
    }
}

impl std::fmt::Debug for ReviewLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewLoop").field("review", &self.review).finish_non_exhaustive()
    }
}

impl App {
    /// Review `last`'s replies through `journal` when the review loop is on; say why not when
    /// it cannot run; do nothing when it is off.
    pub(crate) fn review_in_journal(&mut self, journal: Result<Option<Shared>>) {
        let journal = match journal {
            Ok(None) => return,
            Ok(Some(journal)) => journal,
            Err(err) => return self.loop_off(&err),
        };
        let Some(session) = self.message_session.clone() else {
            return self.loop_off(&anyhow::anyhow!("this agent's session has no id"));
        };
        self.review_loop = Some(ReviewLoop::new(Review::of(journal, &self.message_host, &session)));
        keep_in_journal(self.review_loop.as_mut(), &mut self.open);
        self.strand_pending();
        self.derive_send_state();
        self.picked();
    }

    /// Whether the review loop runs for this review.
    pub(crate) fn review_loop_runs(&self) -> bool {
        self.review_loop.is_some()
    }

    /// Reply `source` as a document: its comments the journal's when the review loop runs.
    pub(super) fn open_reply(&mut self, source: DocumentSource) -> Result<Open> {
        let mut open = Open::new(source, self.open.layout.width, &self.data_dir, &self.project)?;
        keep_in_journal(self.review_loop.as_mut(), &mut open);
        Ok(open)
    }

    /// After the picker opened a reply: say when the journal keeps nothing of it.
    pub(in crate::app) fn picked(&mut self) {
        if self.open.store.is_journal() {
            return;
        }
        let Some(why) = self.review_loop.as_ref().and_then(|review_loop| review_loop.unlisted.clone()) else {
            return;
        };
        self.add_status(&format!("review loop: {why} \u{b7} notes on this reply stay here"));
    }

    /// Say why the review loop is out of this review, after whatever the footer already says.
    fn loop_off(&mut self, err: &anyhow::Error) {
        self.add_status(&format!("review loop off: {err:#} \u{b7} feedback goes to the agent as before"));
    }

    /// `note` after whatever the footer already says.
    fn add_status(&mut self, note: &str) {
        self.status = Some(match self.status.take() {
            Some(status) => format!("{status} \u{b7} {note}"),
            None => note.to_owned(),
        });
    }

    /// Called from the event loop on every wake: read the review's view again after a write,
    /// or when another process (the agent's reply) wrote its journal, and say what it
    /// refused. Returns whether the screen needs a redraw.
    pub(crate) fn poll_journal(&mut self) -> bool {
        // Nothing moves under the person while they type: what came is read once they stop.
        let typing =
            matches!(self.mode, Mode::Compose | Mode::Edit(_) | Mode::Reply(_) | Mode::AttachImage(_));
        if self.review_loop.is_none() || typing {
            return false;
        }
        if self.journal_moved() {
            self.open.store.mark_journal_stale();
        }
        let waiting = self.review_loop.as_ref().and_then(|review_loop| review_loop.failed);
        if !self.open.store.journal_stale() || waiting.is_some_and(|failed| failed.elapsed() < RETRY) {
            return false;
        }
        self.refresh_journal();
        true
    }

    /// The open document's comments as the journal has them now. What it read may be newer
    /// than what the replies the picker keeps aside last read: they read it again when shown.
    fn refresh_journal(&mut self) {
        if self.refresh_path_view() {
            return;
        }
        self.note_stamp();
        for open in self.pick_cache.values_mut() {
            open.store.mark_journal_stale();
        }
        let refused = self.open.store.take_refusals();
        let refreshed = self.open.store.refresh_journal(&self.open.doc, &self.open.layout);
        if refreshed.is_ok() {
            self.rewrite_thread_document();
        }
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.failed = refreshed.is_err().then(Instant::now);
        }
        match refreshed {
            Ok(()) if refused.is_empty() => {}
            Ok(()) => self.status = Some(format!("journal refused: {}", refused.join(" \u{b7} "))),
            Err(err) => self.status = Some(format!("review loop: {err:#}")),
        }
        self.rail_cursor = self.rail_cursor.min(self.open.store.placed().len().saturating_sub(1));
        self.derive_send_state();
        self.paths_changed();
    }
}

/// Give `open`, a reply or a file of the review, the journal's comments when the review loop
/// runs; when the journal cannot list them, its notes stay where they are without the review
/// loop (a reply's in memory, a file's in its record), and `picked` says why for a reply.
fn keep_in_journal(review_loop: Option<&mut ReviewLoop>, open: &mut Open) {
    let Some(review_loop) = review_loop else { return };
    let target = match (&open.source.provenance, &review_loop.review.target) {
        (Provenance::AgentMessage { message_id: Some(message_id), .. }, ReviewTarget::Last { .. }) => {
            if message_id.is_empty() {
                Err(anyhow::anyhow!("this reply has no id to keep comments under"))
            } else {
                Ok(Target::Message { message_id: message_id.clone() })
            }
        }
        (Provenance::File { path }, ReviewTarget::File { .. }) if !open.source.transient => {
            Ok(Target::File { path: real_path(path) })
        }
        _ => return,
    };
    let kept =
        target.and_then(|target| Store::journal(review_loop.review.clone(), target, &open.doc, &open.layout));
    review_loop.unlisted = match kept {
        Ok(store) => {
            open.store = store;
            None
        }
        Err(err) => Some(format!("{err:#}")),
    };
    if review_loop.stamp.is_none() {
        review_loop.stamp = open.store.journal_key().and_then(|key| review_loop.review.journal.stamp(key));
    }
}

#[cfg(test)]
mod tests;
