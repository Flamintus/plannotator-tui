//! What an earlier pane left in the review, handed over when the review loop starts again:
//! the sends the journal wrote and nothing marked delivered (the pane closed before the agent
//! got them), which go again first, by their id; and the drafts the next round carries, which
//! the footer counts, since they may be on a reply or a file not open here.

use plannotator_tui_journal::{Error, IdempotencyKey, Sent, ThreadView};

use super::super::App;
use super::send::Press;
use crate::journal::Review;
use crate::journal::tree::{all_drafts, pending_sends};

impl App {
    /// What an earlier pane left, when the review loop starts: the sends nothing marked
    /// delivered (written, the pane closed before the agent got them), kept to go again
    /// first; and the drafts the next round carries, which the footer counts, since they may
    /// be on a reply or a file not open here.
    pub(in crate::app) fn strand_pending(&mut self) {
        let view = self.open.store.journal_view().map(|(_, view)| view);
        let view = view.or_else(|| self.review_loop.as_ref()?.view.as_ref());
        let mut left = view.map(pending_sends).unwrap_or_default();
        let drafts = view.map_or(0, |view| all_drafts(view).len());
        // The oldest is handed over first.
        left.reverse();
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.left = left;
        }
        if drafts > 0 {
            self.add_status(&format!(
                "review loop: {drafts} draft(s) left by an earlier pane go with the next send"
            ));
        }
    }

    /// The next send an earlier pane left that is still pending in `view` and went to this
    /// review's session, given back by the journal with its text; the others are dropped.
    pub(super) fn left_pending(
        &mut self,
        review: &Review,
        view: &ThreadView,
    ) -> Option<(Press, Result<Sent, Error>)> {
        let pending = pending_sends(view);
        loop {
            let id = self.review_loop.as_mut()?.left.pop()?;
            if !pending.contains(&id) {
                continue;
            }
            let sent = review.journal.resend(&review.target, &id);
            // Only a send a pane of this app wrote (its press's key) to this session: a
            // Plannotator screen's, which its host hands over itself, is not this pane's.
            if let Ok(found) = &sent
                && (review.session().is_some_and(|session| session != found.send.to.session)
                    || !IdempotencyKey::is_terminals(found.send.other.get("idempotency_key")))
            {
                continue;
            }
            return Some((Press::Left(id), sent));
        }
    }
}
