//! The footer's key help over a document the review's journal keeps: only what the selection
//! lets the keys do, first, then, past a wider `·`, the keys that do not change with it (send
//! the round, approve, Tab, send and close, quit, find). The selection is the line the cursor
//! is on in the document (a block, a line of a box's words, a draft's or not, a box's head or
//! folded line), the bubble in the rail, or the tree's row. Whether a key may act there is
//! decided by what the key acts by: the draft whose box the cursor is in (`draft_box`), a
//! thread sent or not (`act::sent`), a note the journal still takes changes to
//! (`Store::journal_waits`), its draft to send at once (`tree::now_drafts`), the fold line
//! selected (`selected_fold`), what `toggle-thread` does here (`toggle_folds`), a draft's words
//! (`words_of_draft`); so the footer names no key that would refuse. The labels say what the key
//! does there: `V reopen` on a resolved thread, `z unfold` where it draws folded threads open.
//! A selection waiting for its action keeps the toolbar's help.

use plannotator_tui_journal::State as ThreadState;

use super::super::{App, Focus};
use super::act::sent;
use crate::gate::Gate;
use crate::journal::tree;
use crate::keys::Action;

impl App {
    /// The footer's two groups under the review loop: the selection's keys, then the keys that
    /// do not change with it. `None` over a document the journal does not keep, or with a
    /// selection waiting for its action: the footer is the pane's, as it always was.
    pub(in crate::app) fn loop_help(&self) -> Option<[Vec<String>; 2]> {
        if !self.open.store.is_journal() || self.pending.is_some() {
            return None;
        }
        let here = match self.focus {
            Focus::Tree => vec![
                self.up_down_hint(),
                self.keys.labelled(Action::Confirm, "open"),
                self.keys.labelled(Action::ToggleHidden, "hidden"),
                self.keys.labelled(Action::ToggleTree, "hide"),
            ],
            Focus::Rail => self.bubble_help(),
            Focus::Document => self.line_help(),
        };
        Some([here.into_iter().flatten().collect(), self.steady_help().into_iter().flatten().collect()])
    }

    /// The keys that do not change with the selection, in the order the panes' help names them.
    fn steady_help(&self) -> Vec<Option<String>> {
        let key = |action, label| self.keys.labelled(action, label);
        let tab = self.keys.hint(Action::CycleFocus);
        vec![
            key(Action::Send, "send"),
            (self.gate == Gate::Approve).then(|| key(Action::Approve, "approve")).flatten(),
            (!tab.is_empty()).then_some(tab),
            // `send-and-close` is bound only in a reply review.
            (!self.is_file_review()).then(|| key(Action::SendAndClose, "send+quit")).flatten(),
            key(Action::Quit, "quit"),
            // Quick Open needs the file tree.
            self.tree.is_some().then(|| key(Action::QuickOpen, "find")).flatten(),
        ]
    }

    /// The rail's bubble: moving, then what its note takes: its words, images and removal while
    /// the journal takes them, then its thread's keys.
    fn bubble_help(&self) -> Vec<Option<String>> {
        let key = |action, label| self.keys.labelled(action, label);
        let note = self.current_note();
        let (waits, images) = self.note_changes(note.as_deref());
        let mut help = vec![
            self.up_down_hint(),
            waits.then(|| key(Action::Edit, "edit")).flatten(),
            waits.then(|| key(Action::Remove, "remove")).flatten(),
            waits.then(|| key(Action::AttachImage, "image")).flatten(),
            images.then(|| key(Action::RemoveImage, "remove image")).flatten(),
        ];
        help.extend(self.thread_help(note.as_deref()));
        help
    }

    /// The document's selected line: on a box's fold line the arrow that folds or unfolds it;
    /// the cursor and a selection where there are words that take one (the document's, a sent
    /// message's); the edit of the draft whose box it is; the images of the current note; its
    /// thread's keys. While the cursor roams, its motions and the way back to the blocks.
    fn line_help(&self) -> Vec<Option<String>> {
        let key = |action, label| self.keys.labelled(action, label);
        let row = self.cursor.0;
        let fold_line = self.on_box_line() && self.fold_line(row).is_some();
        // A box's fold line has no words; a draft's words take no selection.
        let text = !fold_line;
        let takes = text && !self.words_of_draft(row);
        let fold = self.selected_fold().and_then(|(_, folded)| {
            if folded { key(Action::Right, "unfold") } else { key(Action::Left, "fold") }
        });
        let mut help = vec![fold];
        if self.roam {
            let directions = self.keys.directions_hint();
            help.extend([
                (!directions.is_empty()).then(|| format!("{directions} move")),
                takes.then(|| key(Action::Visual, "select")).flatten(),
                takes.then(|| key(Action::Comment, "comment")).flatten(),
                key(Action::Back, "blocks"),
            ]);
        } else {
            help.extend([
                text.then(|| key(Action::MoveCursor, "move")).flatten(),
                takes.then(|| key(Action::Visual, "select")).flatten(),
                takes.then(|| key(Action::Comment, "comment")).flatten(),
            ]);
        }
        help.push(self.draft_box().and_then(|_| key(Action::Edit, "edit")));
        let note = self.current_note();
        if !self.roam {
            let (waits, images) = self.note_changes(note.as_deref());
            help.push(waits.then(|| key(Action::AttachImage, "image")).flatten());
            help.push(images.then(|| key(Action::RemoveImage, "remove image")).flatten());
        }
        help.extend(self.thread_help(note.as_deref()));
        help
    }

    /// Whether note `id` takes changes, its words, its images and its removal, and whether it
    /// has an image to remove: always without the journal, but over the journal's document only
    /// while it waits for a send, the journal refusing them for a sent comment. No note, none.
    fn note_changes(&self, id: Option<&str>) -> (bool, bool) {
        let Some(id) = id else { return (false, false) };
        let waits = self.open.store.journal_waits(id) != Some(false);
        (waits, waits && self.open.store.has_images(id))
    }

    /// The keys on the current note's thread: its draft sent at once, a reply, resolving or
    /// opening it again once sent; then folding or unfolding the threads here.
    fn thread_help(&self, note: Option<&str>) -> Vec<Option<String>> {
        let key = |action, label| self.keys.labelled(action, label);
        let thread = note.and_then(|id| self.node(id));
        let is_sent = thread.is_some_and(sent);
        let resolved = thread.is_some_and(|thread| thread.state == ThreadState::Resolved);
        let now = thread.is_some_and(|thread| !tree::now_drafts(thread).is_empty());
        vec![
            now.then(|| key(Action::SendNow, "send now")).flatten(),
            is_sent.then(|| key(Action::Reply, "reply")).flatten(),
            is_sent.then(|| key(Action::Resolve, if resolved { "reopen" } else { "resolve" })).flatten(),
            self.toggle_folds()
                .and_then(|fold| key(Action::ToggleThread, if fold { "fold" } else { "unfold" })),
        ]
    }
}
