//! Following the journal: on every wake of the event loop, the review's journal is compared
//! with what was last read (its size and time, `Journal::stamp`, no process run), and a
//! journal another process wrote (the agent's reply through `plannotator reply`, a screen of
//! Plannotator's) has its view read again. The view is always read again, never patched.

use super::super::App;

impl App {
    /// Whether the review's journal was written since the view was last read.
    pub(super) fn journal_moved(&mut self) -> bool {
        let cached = || self.pick_cache.values().find_map(|open| open.store.journal_key());
        let Some(key) = self.open.store.journal_key().or_else(cached).map(str::to_owned) else {
            return false;
        };
        let Some(review_loop) = self.review_loop.as_mut() else { return false };
        let stamp = review_loop.review.journal.stamp(&key);
        stamp != std::mem::replace(&mut review_loop.stamp, stamp)
    }

    /// Take the journal as it is now as read: called just before its view is read, so a write
    /// that lands after still counts as new.
    pub(super) fn note_stamp(&mut self) {
        let Some(key) = self.open.store.journal_key().map(str::to_owned) else { return };
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.stamp = review_loop.review.journal.stamp(&key);
        }
    }
}
