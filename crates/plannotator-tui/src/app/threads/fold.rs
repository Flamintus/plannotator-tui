//! Folding by hand, one way for every box, whatever its depth, kind and state
//! (`Flow::fold_by_hand`): the keys, `toggle-thread` and the mouse all go through it. A
//! thread's fold line, a stop of `down` and `up` (`lines`): `right` or `confirm` on its folded
//! line (`▸`) draws it open (on a draft's, `confirm` edits it: `draft_box`), `left` on the head
//! of its box (`▾`) folds it, with the boxes in it back to how they are drawn by default; a
//! click on either does the same, the cursor standing on it as the arrows leave it. The cursor
//! stays on the thread's first row, which turns from one to the other (`boxes_moved`). `toggle-thread`: the folded threads seen on the selected
//! block's passages drawn open, or, when none is folded there, the block's threads folded;
//! with the cursor in a box, the folded threads seen in that box, or that box. `left` on a line
//! of a box's words keeps its meaning: the cursor starts there. Folding resolves nothing and
//! opening reopens nothing.

use super::super::App;
use super::{Flow, Thread};
use crate::keys::Action;

impl Thread {
    /// The threads drawn folded that can be seen: this one, or those under it once it is open.
    fn visible_folded(&self, out: &mut Vec<String>) {
        if self.is_folded() {
            out.push(self.id.clone());
            return;
        }
        for sub in self.sub_threads() {
            sub.visible_folded(out);
        }
    }

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
    /// What `toggle-thread` does here, on the box the cursor is in, else on the selected block's
    /// threads: the folded threads seen there it draws open, or, none folded, those threads it
    /// folds (`true`). `None` with no thread here.
    fn toggled(&self) -> Option<(Vec<String>, bool)> {
        let inside = self.thread_at_cursor();
        let flow = &self.open.flow;
        let here: Vec<&Thread> = match &inside {
            Some(id) => flow.thread(id).into_iter().collect(),
            None => flow
                .threads
                .iter()
                .filter(|(block, _)| *block == self.selected)
                .map(|(_, thread)| thread)
                .collect(),
        };
        if here.is_empty() {
            return None;
        }
        let mut folded = Vec::new();
        for thread in &here {
            thread.visible_folded(&mut folded);
        }
        Some(if folded.is_empty() {
            (here.iter().map(|thread| thread.id.clone()).collect(), true)
        } else {
            (folded, false)
        })
    }

    /// Whether `toggle-thread` folds here (`true`) or draws folded threads open, as the footer
    /// names it; `None` with no thread here, where it does nothing.
    pub(in crate::app) fn toggle_folds(&self) -> Option<bool> {
        self.toggled().map(|(_, fold)| fold)
    }

    /// `toggle-thread`: on the box the cursor is in, else on the selected block's threads, the
    /// folded threads seen there drawn open, or, none folded, those threads folded.
    pub(in crate::app) fn toggle_threads(&mut self) {
        let Some((ids, fold)) = self.toggled() else {
            self.status = Some("no thread on this block".to_owned());
            return;
        };
        let count = ids.len();
        for id in &ids {
            self.open.flow.fold_by_hand(id, fold);
        }
        self.status = Some(if fold {
            format!("{count} thread(s) folded")
        } else {
            format!("{count} folded thread(s) drawn open")
        });
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

    /// `right` or `confirm` on the selected folded line draws its thread open; `left` on the
    /// selected head of a box folds it. The other way, nothing.
    pub(in crate::app) fn fold_key(&mut self, action: Action) {
        let Some((id, folded)) = self.selected_fold() else { return };
        let fold = match action {
            Action::Right | Action::Confirm if folded => false,
            Action::Left if !folded => true,
            _ => return,
        };
        let id = id.to_owned();
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
