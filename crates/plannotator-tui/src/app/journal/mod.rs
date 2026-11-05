//! Reviewing through the Plannotator Inbox (`crate::journal`): an Inbox thread's agent
//! messages and attached files in the picker, each opened as a document whose comments the
//! Inbox keeps (here), the comments sent now or at the end of a round (`send`), the thread
//! followed live on the Inbox's event stream (`watch`), and replies, resolution and
//! sub-threads on the current note's thread (`act`).

mod act;
mod send;
mod watch;

use std::collections::HashSet;

use anyhow::Result;
use plannotator_tui_journal::{ThreadNode, ThreadView};
use plannotator_tui_schema::Provenance;

use super::{App, Mode, Open};
use crate::delivery::Delivery;
use crate::journal::source::{Document, ThreadDocuments};
use crate::journal::{Shared, Target, tree};
use crate::store::Store;

/// What an Inbox review keeps besides its documents.
pub(super) struct InboxReview {
    inbox: Shared,
    /// What each picker candidate opens, by candidate index, when the candidates are an
    /// Inbox thread's documents.
    documents: Vec<Document>,
    /// Why the Inbox could not list the comments of the document opened last.
    unlisted: Option<String>,
    /// The last send that did not reach the Inbox, to try again under its key.
    press: Option<send::Press>,
    /// The thread whose event stream is followed.
    following: Option<watch::Following>,
    /// The agent's replies the person has seen: the ones there when the review first read
    /// the thread, and those under a thread the person acted on since. Others are new.
    seen: HashSet<String>,
    seeded: bool,
    /// The documents left for comments' threads opened as documents, the last one last.
    parents: Vec<act::Parked>,
}

impl InboxReview {
    fn new(inbox: Shared, documents: Vec<Document>) -> Self {
        Self {
            inbox,
            documents,
            unlisted: None,
            press: None,
            following: None,
            seen: HashSet::new(),
            seeded: false,
            parents: Vec::new(),
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

impl std::fmt::Debug for InboxReview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboxReview").field("documents", &self.documents.len()).finish_non_exhaustive()
    }
}

impl App {
    /// Review an Inbox thread: its newest document open, the picker offering them all when
    /// there is more than one.
    pub(crate) fn open_inbox_thread(
        found: ThreadDocuments,
        inbox: Shared,
        width: usize,
        delivery: Box<dyn Delivery>,
    ) -> Result<Self> {
        let Some(first) = found.documents.first() else {
            anyhow::bail!("Inbox thread \"{}\" has no agent message to review", found.subject)
        };
        let mut app =
            Self::open(first.target.source(first.name.clone(), first.text.clone()), width, delivery)?;
        app.candidates = found.documents.iter().map(Document::candidate).collect();
        "Inbox".clone_into(&mut app.message_host);
        if app.candidates.len() > 1 {
            app.mode = Mode::Pick;
        }
        if !found.unreadable.is_empty() {
            app.status = Some(format!("could not read {}", found.unreadable.join("; ")));
        }
        app.inbox = Some(InboxReview::new(inbox, found.documents));
        if let Some(open) = app.inbox_candidate(0)? {
            app.open = open;
        }
        app.say_unlisted();
        Ok(app)
    }

    /// The document picker candidate `index` opens, when the candidates are an Inbox
    /// thread's; `None` for an agent's replies.
    pub(super) fn inbox_candidate(&mut self, index: usize) -> Result<Option<Open>> {
        let Some(review) = &self.inbox else { return Ok(None) };
        let Some(document) = review.documents.get(index).cloned() else { return Ok(None) };
        let inbox = review.inbox.clone();
        let opened =
            self.open_document(&inbox, &document.target, document.name.clone(), document.text.clone());
        let unlisted = opened.as_ref().err().map(|err| format!("{err:#}"));
        if let Some(review) = self.inbox.as_mut() {
            review.unlisted = unlisted;
        }
        match opened {
            Ok(open) => Ok(Some(open)),
            // The Inbox cannot list its comments: its notes stay here, as on any reply.
            Err(_) => self.open_transient(&document.target, document.name, document.text).map(Some),
        }
    }

    /// After the picker opened a document: say when the Inbox keeps nothing of it.
    pub(in crate::app) fn picked(&mut self) {
        self.say_unlisted();
    }

    /// Inbox document `target`, `text` its words, its notes the comments the Inbox lists on
    /// them. An error is the Inbox's: it could not list them.
    fn open_document(&self, inbox: &Shared, target: &Target, name: String, text: String) -> Result<Open> {
        let mut open = self.open_transient(target, name, text)?;
        open.store = Store::inbox(inbox.clone(), target.clone(), &open.doc, &open.layout)?;
        Ok(open)
    }

    fn open_transient(&self, target: &Target, name: String, text: String) -> Result<Open> {
        Open::new(target.source(name, text), self.open.layout.width, &self.data_dir, &self.project)
    }

    /// Say why the open Inbox document keeps its notes here, if it does.
    fn say_unlisted(&mut self) {
        if self.open.store.is_inbox() || !matches!(self.open.source.provenance, Provenance::Inbox { .. }) {
            return;
        }
        let Some(why) = self.inbox.as_ref().and_then(|review| review.unlisted.clone()) else { return };
        self.add_status(&format!("Inbox: {why} \u{b7} notes on this document stay here"));
    }

    /// `note` after whatever the footer already says.
    fn add_status(&mut self, note: &str) {
        self.status = Some(match self.status.take() {
            Some(status) => format!("{status} \u{b7} {note}"),
            None => note.to_owned(),
        });
    }

    /// Called from the event loop on every wake: read the Inbox's view again after a write
    /// or when its stream told of a change, and say what it refused. Returns whether the
    /// screen needs a redraw.
    pub(crate) fn poll_inbox(&mut self) -> bool {
        // Nothing moves under the person while they type: what came is read once they stop.
        let typing =
            matches!(self.mode, Mode::Compose | Mode::Edit(_) | Mode::Reply(_) | Mode::AttachImage(_));
        if self.inbox.is_none() || typing {
            return false;
        }
        let told = self.read_stream();
        if told.changed {
            // The documents the picker keeps aside are read again when they are shown.
            for open in self.pick_cache.values_mut() {
                open.store.mark_inbox_stale();
            }
        }
        if !told.changed && !self.open.store.inbox_stale() {
            return told.said;
        }
        self.refresh_inbox();
        true
    }

    /// The open document's comments as the Inbox has them now.
    fn refresh_inbox(&mut self) {
        let refused = self.open.store.take_refusals();
        let refreshed = self.open.store.refresh_inbox(&self.open.doc, &self.open.layout);
        if refreshed.is_ok() {
            self.rewrite_thread_document();
        }
        match refreshed {
            Ok(()) if refused.is_empty() => {}
            Ok(()) => self.status = Some(format!("Inbox refused: {}", refused.join(" \u{b7} "))),
            Err(err) => self.status = Some(format!("Inbox: {err:#}")),
        }
        self.rail_cursor = self.rail_cursor.min(self.open.store.placed().len().saturating_sub(1));
        self.derive_send_state();
    }
}

#[cfg(test)]
mod tests;
