//! Acting on the current note's thread through the Inbox: `reply` under it, `resolve` it or
//! open it again, `open-thread` to open it as a document, where a passage of one of its
//! messages takes a sub-thread as a comment does anywhere; `back` returns to the document it
//! was opened from. A reply under a live thread is sent by the Inbox on its own.

use anyhow::Result;
use plannotator_tui_journal::{NewReply, State as ThreadState, ThreadNode};

use super::super::draw::short_id;
use super::super::{App, Focus, Mode, Open};
use crate::journal::{Shared, Target, tree};
use crate::keys::Action;

/// A document left for a comment's thread, and where its reader was in it.
pub(super) struct Parked {
    open: Open,
    scroll: usize,
    selected: usize,
    cursor: (usize, usize),
    rail_cursor: usize,
    focus: Focus,
}

impl App {
    /// The thread actions, on the current note: the rail's, or the selected block's.
    pub(in crate::app) fn thread_action(&mut self, action: Action) {
        match action {
            Action::ToggleThread => self.toggle_threads(),
            Action::Reply => self.begin_reply(),
            Action::Resolve => self.resolve_current(),
            Action::OpenThread => self.open_thread(),
            _ => {}
        }
    }

    /// The current note's thread in the Inbox's view, with the Inbox; the footer says why
    /// when there is none.
    fn current_thread(&mut self) -> Option<(Shared, ThreadNode)> {
        let found = self.open.store.inbox_thread().and_then(|(inbox, _, view)| {
            let id = self.current_note()?;
            Some((inbox.clone(), tree::find(&view.threads, &id)?.clone()))
        });
        if found.is_none() {
            self.status = Some(if self.open.store.is_inbox() {
                "no comment here".to_owned()
            } else {
                "replies, resolving and sub-threads go through the Inbox, which does not keep this document's notes"
                    .to_owned()
            });
        }
        found
    }

    fn begin_reply(&mut self) {
        let Some((_, node)) = self.current_thread() else { return };
        if node.draft {
            self.status =
                Some(format!("a draft takes no reply: {} sends it first", self.keys.hint(Action::Send)));
            return;
        }
        self.compose = super::super::compose::Compose::default();
        self.status = None;
        self.mode = Mode::Reply(node.id);
    }

    /// The person's reply `body` under comment `id`; an empty one is no reply.
    pub(in crate::app) fn save_reply(&mut self, id: &str, body: String) {
        if body.is_empty() {
            self.status = Some(format!("reply to {} cancelled: empty", short_id(id)));
            return;
        }
        let Some((inbox, node)) = self.current_thread_of(id) else {
            self.status = Some(format!("reply not saved: {} is no longer in the Inbox", short_id(id)));
            return;
        };
        let reply = NewReply { id: crate::store::local_id(), text: body };
        self.status = Some(match inbox.reply(id, &reply) {
            Ok(_) if node.live => "reply sent: the thread is live".to_owned(),
            Ok(_) => format!("reply saved \u{b7} {} sends it", self.keys.hint(Action::Send)),
            Err(err) => format!("Inbox: {err}"),
        });
        self.saw(&node);
        self.refresh_inbox();
    }

    /// Thread `id` in the open document's view, with the Inbox.
    fn current_thread_of(&self, id: &str) -> Option<(Shared, ThreadNode)> {
        let (inbox, _, view) = self.open.store.inbox_thread()?;
        Some((inbox.clone(), tree::find(&view.threads, id)?.clone()))
    }

    fn resolve_current(&mut self) {
        let Some((inbox, node)) = self.current_thread() else { return };
        let resolve = node.state != ThreadState::Resolved;
        self.status = Some(match inbox.resolve(&node.id, resolve) {
            Ok(_) if resolve => format!("{} resolved", short_id(&node.id)),
            Ok(_) => format!("{} open again", short_id(&node.id)),
            Err(err) => format!("Inbox: {err}"),
        });
        self.saw(&node);
        self.refresh_inbox();
    }

    /// The person acted on `node`'s thread: its replies are no longer new.
    fn saw(&mut self, node: &ThreadNode) {
        if let Some(review) = self.inbox.as_mut() {
            review.saw(node);
        }
    }

    /// Open the current note's thread as a document, the open one kept to come back to.
    fn open_thread(&mut self) {
        let Some((inbox, node)) = self.current_thread() else { return };
        let Some(thread_id) = self.open.store.inbox_thread().map(|(_, thread_id, _)| thread_id.to_owned())
        else {
            return;
        };
        let open = match self.thread_document(&inbox, thread_id, &node) {
            Ok(open) => open,
            Err(err) => {
                self.status = Some(format!("Inbox: {err:#}"));
                return;
            }
        };
        let parked = Parked {
            open: std::mem::replace(&mut self.open, open),
            scroll: self.scroll,
            selected: self.selected,
            cursor: self.cursor,
            rail_cursor: self.rail_cursor,
            focus: self.focus,
        };
        if let Some(review) = self.inbox.as_mut() {
            review.parents.push(parked);
            review.saw(&node);
        }
        (self.scroll, self.selected, self.cursor, self.rail_cursor) = (0, 0, (0, 0), 0);
        self.focus = Focus::Document;
        self.clear_selection();
        self.derive_send_state();
        let back = self.keys.hint(Action::Back);
        let comment = self.keys.hint(Action::Comment);
        self.status = Some(format!(
            "thread {}: select a passage of one message, {comment} starts a sub-thread on it \u{b7} {back} back",
            short_id(&node.id)
        ));
    }

    /// Comment `node`'s thread as a document: its messages, their sub-threads its notes.
    fn thread_document(&self, inbox: &Shared, thread_id: String, node: &ThreadNode) -> Result<Open> {
        let (text, messages) = tree::document(node);
        let target = Target::Thread { thread_id, comment_id: node.id.clone(), messages };
        let quote = node.place.quote.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = format!("thread {} \u{b7} \"{quote}\"", short_id(&node.id));
        self.open_document(inbox, &target, name, text)
    }

    /// In a comment's thread opened as a document: whether one was left for it.
    pub(in crate::app) fn in_thread_document(&self) -> bool {
        self.inbox.as_ref().is_some_and(|review| !review.parents.is_empty())
    }

    /// `back` in a comment's thread: the document it was opened from, as it was left, its
    /// notes read again. False when no document was left for this one.
    pub(in crate::app) fn leave_thread_document(&mut self) -> bool {
        let Some(parked) = self.inbox.as_mut().and_then(|review| review.parents.pop()) else { return false };
        self.open = parked.open;
        (self.scroll, self.selected, self.cursor, self.rail_cursor) =
            (parked.scroll, parked.selected, parked.cursor, parked.rail_cursor);
        self.focus = parked.focus;
        self.clear_selection();
        self.open.store.mark_inbox_stale();
        self.status = None;
        true
    }

    /// A comment's thread opened as a document, written again from the view when its
    /// messages changed (a reply came), the reader kept where they were.
    pub(super) fn rewrite_thread_document(&mut self) {
        let Some(Target::Thread { thread_id, comment_id, .. }) = self.open.store.inbox_target().cloned()
        else {
            return;
        };
        let Some((inbox, node)) = self.current_thread_of(&comment_id) else { return };
        if tree::document(&node).0 == self.open.doc.source {
            return;
        }
        match self.thread_document(&inbox, thread_id, &node) {
            Ok(open) => {
                self.open = open;
                self.clear_selection();
            }
            Err(err) => self.status = Some(format!("Inbox: {err:#}")),
        }
    }

    /// What a note's bubble title adds over an Inbox document: not saved, its draft, or what
    /// became of its send.
    pub(in crate::app) fn inbox_tag(&self, id: &str) -> String {
        if self.open.store.inbox_unsaved(id) {
            return " \u{b7} not saved".to_owned();
        }
        let Some((_, _, view)) = self.open.store.inbox_thread() else { return String::new() };
        let Some(node) = tree::find(&view.threads, id).filter(|node| node.state != ThreadState::Resolved)
        else {
            return String::new();
        };
        let status = node.status.and_then(|status| serde_json::to_value(status).ok());
        match (node.draft, status.as_ref().and_then(serde_json::Value::as_str)) {
            (true, _) => " \u{b7} draft".to_owned(),
            (false, Some(status)) if status != "unknown" => format!(" \u{b7} {status}"),
            _ => String::new(),
        }
    }

    /// The thread keys the footer adds over an Inbox document, with nothing selected.
    pub(in crate::app) fn inbox_help(&self) -> Vec<Option<String>> {
        if !self.open.store.is_inbox() || self.pending.is_some() || self.focus == Focus::Tree {
            return Vec::new();
        }
        let back = self.in_thread_document().then(|| self.keys.labelled(Action::Back, "back")).flatten();
        vec![
            self.keys.labelled(Action::Reply, "reply"),
            self.keys.labelled(Action::Resolve, "resolve"),
            self.keys.labelled(Action::OpenThread, "thread"),
            self.keys.labelled(Action::ToggleThread, "fold"),
            self.keys.labelled(Action::FinishReview, "finish"),
            back,
        ]
    }
}
