//! The keys over a document the review's journal keeps: what the selection lets them do, one
//! list (`selection_keys`) that the footer names first and that every key whose action depends
//! on the selection (`DECIDED`) consults before acting (`applies`), so the footer and the keys
//! never differ: such a key the footer does not name does nothing, says nothing, opens nothing;
//! the others (moves, `clear-all`…) act as they always did. Past a `|`, the footer
//! names the keys that do not change with the selection (send the round, approve, Tab, send
//! and close, quit, find). The selection is the line the cursor is on in the document (a block,
//! a line of a box's words, a draft's or not, a box's head or folded line), the bubble in the
//! rail, or the tree's row.
//!
//! A comment's keys (its edit, removal and images, a reply, resolving it, sending it at once,
//! folding) act only on the box the cursor is in, at any depth, or on the bubble in the rail,
//! that comment's alone, never from a line of the document, which is not a comment: a reply answers a passage sent,
//! not the words that hold it. Whether a key may act there is decided by what it acts by: the
//! draft whose box the cursor is in (`draft_box`), a thread sent or not (`act::sent`), a note
//! the journal still takes changes to (`Store::journal_waits`), its draft to send at once
//! (`tree::now_drafts`), the fold line selected (`selected_fold`), what `toggle-thread` does
//! here (`fold_keys`), a draft's words (`words_of_draft`). The labels say what the key does
//! there: `V reopen` on a resolved thread, `f unfold` on a folded box. One effect is one offer,
//! named by one key (`f fold`), the other keys that do it there acting as its aliases (the
//! arrow of a fold line). The footer draws each key in the key colour and what it does dim
//! (`Hint`). A selection waiting for its action keeps the toolbar's help, the keys still
//! deciding as the selection lets them.

use plannotator_tui_journal::State as ThreadState;

use super::super::{App, Focus};
use super::act::sent;
use crate::gate::Gate;
use crate::journal::tree;
use crate::keys::Action;

/// One entry of the footer's help under the review loop: the keys, in the user's binding, and
/// what they do there, if the keys do not say it (`tab`, `j/k`).
pub(in crate::app) struct Hint {
    pub(in crate::app) keys: String,
    pub(in crate::app) label: Option<&'static str>,
}

/// A key the selection lets act, as the footer names it.
enum Offer {
    /// `action`, named by what it does on the selection.
    Key(Action, &'static str),
    /// The keys that move, named together (`j/k`, the cursor's directions and `move`).
    Moves(Option<Hint>),
    /// `action`, acting as a key the footer names, not named itself: `confirm` as `edit`, as
    /// the unfold of a folded line or as `comment`; `left` and `right` as `move-cursor`, or as
    /// the fold of a fold line; or acting as it always did, unnamed: `clear-block` on a line of
    /// the document.
    Alias(Action),
}

/// The keys whose action depends on the selection: over a document the journal keeps, each
/// acts only where the selection's keys name it, or as an alias of one; every other key acts
/// as it always did.
const DECIDED: &[Action] = &[
    Action::Edit,
    Action::Remove,
    Action::ClearBlock,
    Action::AttachImage,
    Action::RemoveImage,
    Action::Reply,
    Action::Resolve,
    Action::SendNow,
    Action::ToggleThread,
    Action::MoveCursor,
    Action::Visual,
    Action::Comment,
    Action::Confirm,
    Action::Left,
    Action::Right,
];

impl App {
    /// The footer's two groups under the review loop: the selection's keys, then the keys that
    /// do not change with it. `None` over a document the journal does not keep, or with a
    /// selection waiting for its action: the footer is the pane's, as it always was.
    pub(in crate::app) fn loop_help(&self) -> Option<[Vec<Hint>; 2]> {
        if !self.open.store.is_journal() || self.pending.is_some() {
            return None;
        }
        let here = self.selection_keys().into_iter().filter_map(|offer| match offer {
            Offer::Key(action, label) => self.hint(action, label),
            Offer::Moves(hint) => hint,
            Offer::Alias(_) => None,
        });
        Some([here.collect(), self.steady_help().into_iter().flatten().collect()])
    }

    /// `action`'s first key and what it does, or none when it is unbound.
    fn hint(&self, action: Action, label: &'static str) -> Option<Hint> {
        let keys = self.keys.hint(action);
        (!keys.is_empty()).then_some(Hint { keys, label: Some(label) })
    }

    /// Whether `action` may act on the selection: over a document the journal keeps, a key
    /// whose action depends on the selection acts only where the footer names it, or as an
    /// alias of one; any other key, and every key over another document, as it always did.
    pub(in crate::app) fn applies(&self, action: Action) -> bool {
        if !DECIDED.contains(&action) || !self.open.store.is_journal() {
            return true;
        }
        self.selection_keys().iter().any(|offer| match offer {
            Offer::Key(named, _) | Offer::Alias(named) => *named == action,
            Offer::Moves(_) => false,
        })
    }

    /// What the selection lets the keys do, in the order the footer names them.
    fn selection_keys(&self) -> Vec<Offer> {
        match self.focus {
            Focus::Tree => vec![
                Offer::Moves(self.up_down_hint().map(|keys| Hint { keys, label: None })),
                Offer::Key(Action::Confirm, "open"),
                Offer::Key(Action::ToggleHidden, "hidden"),
                Offer::Key(Action::ToggleTree, "hide"),
            ],
            Focus::Rail => self.bubble_keys(),
            Focus::Document => self.line_keys(),
        }
    }

    /// The keys that do not change with the selection, in the order the panes' help names them.
    fn steady_help(&self) -> Vec<Option<Hint>> {
        let key = |action, label| self.hint(action, label);
        let tab = self.keys.hint(Action::CycleFocus);
        vec![
            key(Action::Send, "send"),
            (self.gate == Gate::Approve).then(|| key(Action::Approve, "approve")).flatten(),
            (!tab.is_empty()).then_some(Hint { keys: tab, label: None }),
            // `send-and-close` is bound only in a reply review.
            (!self.is_file_review()).then(|| key(Action::SendAndClose, "send+quit")).flatten(),
            key(Action::Quit, "quit"),
            // Quick Open needs the file tree.
            self.tree.is_some().then(|| key(Action::QuickOpen, "find")).flatten(),
        ]
    }

    /// The rail's bubble: moving, then what its note takes: its words (`confirm` too), images
    /// and removal while the journal takes them, then its thread's keys.
    fn bubble_keys(&self) -> Vec<Offer> {
        let note = self.current_note();
        let (waits, images) = self.note_changes(note.as_deref());
        let mut keys = vec![Offer::Moves(self.up_down_hint().map(|keys| Hint { keys, label: None }))];
        if waits {
            keys.extend([
                Offer::Key(Action::Edit, "edit"),
                Offer::Alias(Action::Confirm),
                Offer::Key(Action::Remove, "remove"),
                Offer::Key(Action::AttachImage, "image"),
            ]);
        }
        if images {
            keys.push(Offer::Key(Action::RemoveImage, "remove image"));
        }
        keys.extend(self.thread_keys(note.as_deref()));
        keys
    }

    /// The document's selected line: the cursor and a selection where there are words that take one (the document's, a sent
    /// message's), while the cursor roams its motions and the way back to the blocks; then, in
    /// a box only, its note's keys: the edit of the draft whose box it is, its removal by
    /// `clear-block` while the journal takes it, its images, its thread's keys; on a line of
    /// the document `clear-block` clears the block, as it always did. `confirm` acts as the first of edit, unfold and comment the line takes.
    fn line_keys(&self) -> Vec<Offer> {
        let row = self.cursor.0;
        let fold_line = self.on_box_line() && self.fold_line(row).is_some();
        // A box's fold line has no words; a draft's words take no selection.
        let text = !fold_line;
        let takes = text && !self.words_of_draft(row);
        let fold = self.selected_fold().map(|(_, folded)| folded);
        let draft = self.draft_box().is_some();
        let mut keys = Vec::new();
        if self.roam {
            let directions = self.keys.directions_hint();
            keys.push(Offer::Moves(
                (!directions.is_empty()).then_some(Hint { keys: directions, label: Some("move") }),
            ));
        } else if text {
            // A column move in block mode starts the cursor, as `move-cursor` does.
            keys.extend([
                Offer::Key(Action::MoveCursor, "move"),
                Offer::Alias(Action::Left),
                Offer::Alias(Action::Right),
            ]);
        }
        if takes {
            keys.extend([Offer::Key(Action::Visual, "select"), Offer::Key(Action::Comment, "comment")]);
        }
        if self.roam {
            keys.push(Offer::Key(Action::Back, "blocks"));
        }
        if draft || fold.unwrap_or(takes) {
            keys.push(Offer::Alias(Action::Confirm));
        }
        // A comment's keys act on the box the cursor is in, never on a line of the document.
        let Some(note) = self.thread_at_cursor() else {
            keys.push(Offer::Alias(Action::ClearBlock));
            return keys;
        };
        if draft {
            keys.push(Offer::Key(Action::Edit, "edit"));
        }
        let (waits, images) = self.note_changes(Some(&note));
        if waits {
            keys.extend([Offer::Key(Action::ClearBlock, "remove"), Offer::Key(Action::AttachImage, "image")]);
        }
        if images {
            keys.push(Offer::Key(Action::RemoveImage, "remove image"));
        }
        keys.extend(self.thread_keys(Some(&note)));
        keys
    }

    /// Whether note `id` takes changes, its words, its images and its removal, and whether it
    /// has an image to remove: always without the journal, but over the journal's document only
    /// while it waits for a send, the journal refusing them for a sent comment. No note, none.
    fn note_changes(&self, id: Option<&str>) -> (bool, bool) {
        let Some(id) = id else { return (false, false) };
        let waits = self.open.store.journal_waits(id) != Some(false);
        (waits, waits && self.open.store.has_images(id))
    }

    /// The keys on note `note`'s thread: its draft sent at once, a reply, resolving or opening
    /// it again once sent; then folding or unfolding the threads here.
    fn thread_keys(&self, note: Option<&str>) -> Vec<Offer> {
        let thread = note.and_then(|id| self.node(id));
        let mut keys = Vec::new();
        if thread.is_some_and(|thread| !tree::now_drafts(thread).is_empty()) {
            keys.push(Offer::Key(Action::SendNow, "send now"));
        }
        if let Some(thread) = thread.filter(|thread| sent(thread)) {
            let resolved = thread.state == ThreadState::Resolved;
            keys.push(Offer::Key(Action::Reply, "reply"));
            keys.push(Offer::Key(Action::Resolve, if resolved { "reopen" } else { "resolve" }));
        }
        if let Some((fold, aliases, folded)) = self.fold_keys() {
            keys.push(Offer::Key(fold, if folded { "unfold" } else { "fold" }));
            keys.extend(aliases.into_iter().map(Offer::Alias));
        }
        keys
    }
}
