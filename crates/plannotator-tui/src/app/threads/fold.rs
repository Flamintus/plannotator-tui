//! Folding by hand, one way for every box, whatever its depth, kind and state
//! (`Flow::fold_by_hand`), decided in one place: the box folding acts on here (`fold_here`, the
//! box the cursor is in or the bubble's thread in the rail) and the keys that fold or unfold it
//! there (`fold_keys`), which the footer names as one offer and which act through one function
//! (`fold_key`). `toggle-thread` folds or unfolds that box, wherever in it the cursor is; on its
//! fold line, a stop of `down` and `up` (`lines`), `left` folds its head (`▾`) and `right` or
//! `confirm` draws its folded line (`▸`) open (on a draft's, `confirm` edits it: `draft_box`);
//! a click on either does the same, the cursor standing on it as the arrows leave it. Folded,
//! the boxes in it go back to how they are drawn by default. The cursor stays on the thread's
//! first row, which turns from one to the other (`boxes_moved`). `left` on a line of a box's
//! words keeps its meaning: the cursor starts there. Folding resolves nothing and opening
//! reopens nothing.

use super::super::{App, Focus};
use super::{Flow, Thread};
use crate::keys::Action;

impl Thread {
    /// This thread, or the one under it at any depth, by id.
    fn find(&self, id: &str) -> Option<&Self> {
        if self.id == id {
            return Some(self);
        }
        self.sub_threads().find_map(|sub| sub.find(id))
    }

    /// The ids of every thread under it, at any depth.
    fn ids_under(&self, out: &mut Vec<String>) {
        for sub in self.sub_threads() {
            out.push(sub.id.clone());
            sub.ids_under(out);
        }
    }
}

impl Flow {
    /// Thread `id`, at any depth.
    pub(super) fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find_map(|(_, thread)| thread.find(id))
    }

    /// Fold thread `id` by hand, or draw it open: the one way for every box, at any depth, of
    /// any kind, resolved or not. Folded, the boxes in it go back to how they are drawn by
    /// default; a thread drawn as by default is no longer kept as by hand, so a resolution
    /// changes how it is drawn as it would have. The rows follow at the next frame.
    fn fold_by_hand(&mut self, id: &str, fold: bool) {
        let Some(thread) = self.thread(id) else { return };
        let folds = thread.folds();
        if fold {
            let mut under = Vec::new();
            thread.ids_under(&mut under);
            for sub in &under {
                self.by_hand.remove(sub);
            }
        }
        if fold == folds {
            self.by_hand.remove(id);
        } else {
            self.by_hand.insert(id.to_owned(), fold);
        }
    }
}

impl App {
    /// The box folding acts on here: the bubble's thread in the rail, else the box the cursor is
    /// in, at any depth, with whether it is drawn folded. `None` with no box here: a comment's
    /// keys act in its box or on its bubble, never from a line of the document.
    fn fold_here(&self) -> Option<(String, bool)> {
        let id = if self.focus == Focus::Rail { self.current_note() } else { self.thread_at_cursor() }?;
        let folded = self.open.flow.thread(&id)?.is_folded();
        Some((id, folded))
    }

    /// The keys that fold the box here, or draw it open, and whether it is folded: the one offer
    /// the footer names, `toggle-thread`, with the keys that do the same there as its aliases
    /// (on its selected fold line, `left` on its head, `right` on its folded line). `None` with
    /// no box here.
    pub(in crate::app) fn fold_keys(&self) -> Option<(Action, Vec<Action>, bool)> {
        let (_, folded) = self.fold_here()?;
        let mut aliases = Vec::new();
        if self.focus == Focus::Document && self.selected_fold().is_some() {
            aliases.push(if folded { Action::Right } else { Action::Left });
        }
        Some((Action::ToggleThread, aliases, folded))
    }

    /// The fold line at document row `row`, while the boxes' words take a selection: its
    /// thread's id, and whether the thread is folded (its folded line) or open (its head).
    pub(in crate::app) fn fold_line(&self, row: usize) -> Option<(&str, bool)> {
        if !self.box_words_selectable() {
            return None;
        }
        let (_, drawn) = self.box_row(row)?;
        Some((drawn.thread.as_deref()?, drawn.fold?))
    }

    /// The fold line selected as a block is (`box_line_selected`), where `right`, `confirm`
    /// and `left` fold or unfold its thread.
    pub(in crate::app) fn selected_fold(&self) -> Option<(&str, bool)> {
        self.box_line_selected().then(|| self.fold_line(self.cursor.0)).flatten()
    }

    /// A key that folds or unfolds the box here (`fold_keys`), the one way the keys do it:
    /// `toggle-thread` folds it or draws it open; on its selected fold line, `left` folds its
    /// head, `right` or `confirm` draws its folded line open. The other way, nothing.
    pub(in crate::app) fn fold_key(&mut self, action: Action) {
        let Some((id, folded)) = self.fold_here() else { return };
        let on_fold_line = self.focus == Focus::Document && self.selected_fold().is_some();
        let fold = match action {
            Action::ToggleThread => !folded,
            Action::Right | Action::Confirm if on_fold_line && folded => false,
            Action::Left if on_fold_line && !folded => true,
            _ => return,
        };
        self.open.flow.fold_by_hand(&id, fold);
    }

    /// A click on the fold line at document row `row`: the cursor stands on it, as `down` and
    /// `up` leave it, and its thread opens if folded, else folds. Whether there was one: a
    /// click anywhere else places the cursor and starts a selection, as it always did.
    pub(in crate::app) fn fold_clicked(&mut self, row: usize) -> bool {
        let Some((id, folded)) = self.fold_line(row) else { return false };
        let id = id.to_owned();
        self.stand_on_line(row);
        self.open.flow.fold_by_hand(&id, !folded);
        true
    }
}
