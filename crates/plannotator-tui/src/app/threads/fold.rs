//! `toggle-thread`: the folded threads on the selected block's passages drawn open, or,
//! when none is folded there, the ones drawn open by hand folded again; with the cursor in a
//! box, the folded threads in that box, or that box and the ones in it.

use super::super::App;
use super::Thread;

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

impl App {
    pub(in crate::app) fn toggle_threads(&mut self) {
        let inside = self.thread_at_cursor();
        let flow = &mut self.open.flow;
        let here: Vec<&Thread> = match &inside {
            Some(id) => flow.threads.iter().find_map(|(_, thread)| thread.find(id)).into_iter().collect(),
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
            flow.unfolded.extend(folded);
            self.status = Some(format!("{count} folded thread(s) drawn open"));
            return;
        }
        let mut ids = Vec::new();
        for thread in &here {
            thread.ids(&mut ids);
        }
        let before = flow.unfolded.len();
        flow.unfolded.retain(|id| !ids.contains(id));
        self.status = Some(if flow.unfolded.len() < before {
            "folded again".to_owned()
        } else if inside.is_some() {
            "no folded thread in this box".to_owned()
        } else {
            "no folded thread on this block".to_owned()
        });
    }
}
