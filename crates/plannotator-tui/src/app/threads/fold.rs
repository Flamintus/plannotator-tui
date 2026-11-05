//! Folding by hand, one way for every thread at any depth. `toggle-thread`: the folded threads
//! on the selected block's passages drawn open, or, when none is folded there, the ones drawn
//! open by hand folded again; with the cursor in a box, the folded threads in that box, or that
//! box and the ones in it. A thread's fold line, a stop of `down` and `up` (`lines`): `right`
//! or `confirm` on its folded line (`▸`) draws it open to be read, still resolved; `left` on
//! the head of its box drawn open by hand (`▾`) folds it again, with the ones drawn open in it.
//! The cursor stays on the thread's first row, which turns from one to the other
//! (`boxes_moved`). `left` on a line of a box's words keeps its meaning: the cursor starts there.

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
        for sub in self.messages.iter().flat_map(|message| &message.threads) {
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

    /// Its id and the ids of every thread under it.
    fn ids(&self, out: &mut Vec<String>) {
        out.push(self.id.clone());
        for sub in self.messages.iter().flat_map(|message| &message.threads) {
            sub.ids(out);
        }
    }
}

impl Flow {
    /// Thread `id`, at any depth.
    fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find_map(|(_, thread)| thread.find(id))
    }

    /// Draw open by hand the folded threads `ids` names.
    fn draw_open(&mut self, ids: Vec<String>) {
        self.unfolded.extend(ids);
    }

    /// Fold again the threads `ids` names drawn open by hand: whether any was.
    fn fold_again(&mut self, ids: &[String]) -> bool {
        let before = self.unfolded.len();
        self.unfolded.retain(|id| !ids.contains(id));
        self.unfolded.len() < before
    }
}

impl App {
    pub(in crate::app) fn toggle_threads(&mut self) {
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
        let mut folded = Vec::new();
        for thread in &here {
            thread.visible_folded(&mut folded);
        }
        if !folded.is_empty() {
            let count = folded.len();
            self.open.flow.draw_open(folded);
            self.status = Some(format!("{count} folded thread(s) drawn open"));
            return;
        }
        let mut ids = Vec::new();
        for thread in &here {
            thread.ids(&mut ids);
        }
        let refolded = self.open.flow.fold_again(&ids);
        self.status = Some(if refolded {
            "folded again".to_owned()
        } else if inside.is_some() {
            "no folded thread in this box".to_owned()
        } else {
            "no folded thread on this block".to_owned()
        });
    }

    /// The fold line at document row `row`, while the boxes' words take a selection: its
    /// thread's id, and whether the thread is folded (its folded line) or drawn open by hand
    /// (its head).
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
    /// selected head of a thread drawn open by hand folds it again. The other way, nothing.
    pub(in crate::app) fn fold_key(&mut self, action: Action) {
        let Some((id, folded)) = self.selected_fold() else { return };
        let id = id.to_owned();
        let flow = &mut self.open.flow;
        match action {
            Action::Right | Action::Confirm if folded => flow.draw_open(vec![id]),
            Action::Left if !folded => {
                let mut ids = Vec::new();
                if let Some(thread) = flow.thread(&id) {
                    thread.ids(&mut ids);
                }
                flow.fold_again(&ids);
            }
            _ => {}
        }
    }
}
