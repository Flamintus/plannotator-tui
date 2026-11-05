//! Acting on the current note's thread through the journal: `reply` under it, `resolve` it or
//! open it again, `open-thread` to open it as a document, where a passage of one of its
//! messages takes a sub-thread as a comment does anywhere; `back` returns to the document it
//! was opened from; `toggle-thread` draws a folded thread open, or folds it again. These keys
//! answer only while the review loop runs. A passage selected in a message's words right in
//! its box takes a sub-thread too (`save_sub_thread`).

use anyhow::Result;
use plannotator_tui_journal::{NewComment, Placement, State as ThreadState, ThreadNode};
use plannotator_tui_schema::{DocumentSource, Provenance};

use super::super::draw::short_id;
use super::super::threads::Passage;
use super::super::{App, Focus, Mode, Open};
use crate::journal::{Review, Target, tree};
use crate::keys::Action;
use crate::store::Store;

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
    /// The thread actions, on the current note: the rail's, or the selected block's. Without
    /// the review loop they do nothing.
    pub(in crate::app) fn thread_action(&mut self, action: Action) {
        if self.review_loop.is_none() {
            return;
        }
        match action {
            Action::ToggleThread => self.toggle_threads(),
            Action::Reply => self.begin_reply(),
            Action::Resolve => self.resolve_current(),
            Action::OpenThread => self.open_thread(),
            _ => {}
        }
    }

    /// The current note's thread in the review's view, with the review; the footer says why
    /// when there is none.
    fn current_thread(&mut self) -> Option<(Review, ThreadNode)> {
        let found = self.open.store.journal_view().and_then(|(review, view)| {
            let id = self.current_note()?;
            Some((review.clone(), tree::find(&view.threads, &id)?.clone()))
        });
        if found.is_none() {
            self.status = Some(if self.open.store.is_journal() {
                "no comment here".to_owned()
            } else {
                "replies, resolving and sub-threads go through the review's journal, which does not keep this document's notes"
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

    /// The person's reply `body` under comment `id`, a draft the next send carries; an empty
    /// one is no reply.
    pub(in crate::app) fn save_reply(&mut self, id: &str, body: String) {
        if body.is_empty() {
            self.status = Some(format!("reply to {} cancelled: empty", short_id(id)));
            return;
        }
        let Some((review, node)) = self.thread_of(id) else {
            self.status = Some(format!("reply not saved: {} is no longer in the review", short_id(id)));
            return;
        };
        let reply = NewComment {
            id: crate::store::local_id(),
            text: body,
            placement: Placement::Under { re: id.to_owned() },
        };
        self.status = Some(match review.journal.comment(&review.target, &reply) {
            Ok(()) => format!("reply saved \u{b7} {} sends it", self.keys.hint(Action::Send)),
            Err(err) => format!("review loop: {err}"),
        });
        self.saw(&node);
        self.refresh_journal();
    }

    /// The person's sub-thread `body` on `passage`, selected in a message's words in its box:
    /// a draft the next send carries, as one made in the thread opened as a document is.
    pub(in crate::app) fn save_sub_thread(&mut self, passage: &Passage, body: String) {
        let Some((review, view)) = self.open.store.journal_view() else {
            self.status =
                Some("sub-thread not saved: the review's journal does not keep this document".to_owned());
            return;
        };
        let review = review.clone();
        let holder = tree::holding(&view.threads, &passage.parent).cloned();
        let comment = NewComment {
            id: crate::store::local_id(),
            text: body,
            placement: Placement::Passage {
                parent: passage.parent.clone(),
                quote: passage.quote.clone(),
                quote_offset: passage.offset,
            },
        };
        self.status = Some(match review.journal.comment(&review.target, &comment) {
            Ok(()) => format!("sub-thread saved \u{b7} {} sends it", self.keys.hint(Action::Send)),
            Err(err) => format!("review loop: {err}"),
        });
        if let Some(node) = holder {
            self.saw(&node);
        }
        self.refresh_journal();
    }

    /// Thread `id` in the open document's view, with the review.
    fn thread_of(&self, id: &str) -> Option<(Review, ThreadNode)> {
        let (review, view) = self.open.store.journal_view()?;
        Some((review.clone(), tree::find(&view.threads, id)?.clone()))
    }

    fn resolve_current(&mut self) {
        let Some((review, node)) = self.current_thread() else { return };
        let resolve = node.state != ThreadState::Resolved;
        self.status = Some(match review.journal.resolve(&review.target, &node.id, resolve) {
            Ok(()) if resolve => format!("{} resolved", short_id(&node.id)),
            Ok(()) => format!("{} open again", short_id(&node.id)),
            Err(err) => format!("review loop: {err}"),
        });
        self.saw(&node);
        self.refresh_journal();
    }

    /// The person acted on `node`'s thread: its replies are no longer new.
    fn saw(&mut self, node: &ThreadNode) {
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.saw(node);
        }
    }

    /// Open the current note's thread as a document, the open one kept to come back to.
    fn open_thread(&mut self) {
        let Some((review, node)) = self.current_thread() else { return };
        let open = match self.thread_document(review, &node) {
            Ok(open) => open,
            Err(err) => {
                self.status = Some(format!("review loop: {err:#}"));
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
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.parents.push(parked);
            review_loop.saw(&node);
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
    fn thread_document(&self, review: Review, node: &ThreadNode) -> Result<Open> {
        let (text, messages) = tree::document(node);
        let target = Target::Thread { comment_id: node.id.clone(), messages };
        let quote = node.place.quote.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = format!("thread {} \u{b7} \"{quote}\"", short_id(&node.id));
        let source = DocumentSource::new(text, name, true, Provenance::Stdin);
        let mut open = Open::new(source, self.open.layout.width, &self.data_dir, &self.project)?;
        open.store = Store::journal(review, target, &open.doc, &open.layout)?;
        Ok(open)
    }

    /// In a comment's thread opened as a document: whether one was left for it.
    pub(in crate::app) fn in_thread_document(&self) -> bool {
        self.review_loop.as_ref().is_some_and(|review_loop| !review_loop.parents.is_empty())
    }

    /// The source a send is archived under: the open document's, or, in a comment's thread
    /// opened as a document, the reply it was opened from.
    pub(in crate::app) fn archived_provenance(&self) -> &Provenance {
        let first = self.review_loop.as_ref().and_then(|review_loop| review_loop.parents.first());
        first.map_or(&self.open.source.provenance, |parked| &parked.open.source.provenance)
    }

    /// `back` in a comment's thread: the document it was opened from, as it was left, its
    /// notes read again. False when no document was left for this one.
    pub(in crate::app) fn leave_thread_document(&mut self) -> bool {
        let Some(parked) = self.review_loop.as_mut().and_then(|review_loop| review_loop.parents.pop()) else {
            return false;
        };
        self.open = parked.open;
        (self.scroll, self.selected, self.cursor, self.rail_cursor) =
            (parked.scroll, parked.selected, parked.cursor, parked.rail_cursor);
        self.focus = parked.focus;
        self.clear_selection();
        self.open.store.mark_journal_stale();
        self.status = None;
        true
    }

    /// A comment's thread opened as a document, written again from the view when its
    /// messages changed (a reply came), the reader kept where they were.
    pub(super) fn rewrite_thread_document(&mut self) {
        let Some(Target::Thread { comment_id, .. }) = self.open.store.journal_target().cloned() else {
            return;
        };
        let Some((review, node)) = self.thread_of(&comment_id) else { return };
        if tree::document(&node).0 == self.open.doc.source {
            return;
        }
        match self.thread_document(review, &node) {
            Ok(open) => {
                let old = std::mem::replace(&mut self.open, open);
                self.open.store.carry_over(&old.store, &self.open.doc);
                self.clear_selection();
            }
            Err(err) => self.status = Some(format!("review loop: {err:#}")),
        }
    }

    /// What a note's bubble title adds over a document of the journal: not saved, its draft,
    /// or what became of its send.
    pub(in crate::app) fn journal_tag(&self, id: &str) -> String {
        if self.open.store.journal_unsaved(id) {
            return " \u{b7} not saved".to_owned();
        }
        let Some((_, view)) = self.open.store.journal_view() else { return String::new() };
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

    /// The thread keys the footer adds over a document of the journal, with nothing selected.
    pub(in crate::app) fn journal_help(&self) -> Vec<Option<String>> {
        if !self.open.store.is_journal() || self.pending.is_some() || self.focus == Focus::Tree {
            return Vec::new();
        }
        let back = self.in_thread_document().then(|| self.keys.labelled(Action::Back, "back")).flatten();
        vec![
            self.keys.labelled(Action::Reply, "reply"),
            self.keys.labelled(Action::Resolve, "resolve"),
            self.keys.labelled(Action::OpenThread, "thread"),
            self.keys.labelled(Action::ToggleThread, "fold"),
            back,
        ]
    }
}
