//! Acting on the current note's thread through the journal: `reply` under it, `resolve` it or
//! open it again, `send-now` sends its draft at once (`send`), `toggle-live` turns Live on or
//! off on it, turning it on sending first, at once, what waits in its box, a reply saved
//! under a Live thread going at once by `send-now`'s own
//! send, `toggle-thread` draws a folded thread open, or folds an open one, as `right` and
//! `left` do on its fold line, whatever the box (`threads::fold`). These keys answer only
//! while the review loop runs. A sub-thread has one way in: a passage selected in a message's
//! words right in its box, noted as the document's words are (`App::annotate`). On any line of
//! a draft's box, `edit` and `confirm` edit it there (`draft_box`), as `edit` does on its
//! bubble.

use plannotator_tui_journal::{NewComment, Placement, State as ThreadState, ThreadNode};

use super::super::draw::short_id;
use super::super::{App, Mode};
use crate::journal::{Review, tree};
use crate::keys::Action;

/// Whether thread `node` takes a reply, a resolution and Live: once its comment is sent. On a
/// draft the keys offer none (`help`), and do nothing: it is sent first.
pub(super) fn sent(node: &ThreadNode) -> bool {
    !node.draft
}

impl App {
    /// The thread actions, on the current note: the rail's, or the box the cursor is in (the
    /// keys act nowhere else over the journal's document: `help`). Without the review loop
    /// they do nothing.
    pub(in crate::app) fn thread_action(&mut self, action: Action) {
        if self.review_loop.is_none() {
            return;
        }
        match action {
            Action::ToggleThread => self.fold_key(Action::ToggleThread),
            Action::Reply => self.begin_reply(),
            Action::Resolve => self.resolve_current(),
            Action::SendNow => self.send_current(),
            Action::ToggleLive => self.toggle_live(),
            _ => {}
        }
    }

    /// The current note's thread in the review's view, with the review (`current_note`: the
    /// box the cursor is in or the rail's note, where the keys offer it). The footer says why
    /// when the journal does not keep the document's notes.
    fn current_thread(&mut self) -> Option<(Review, ThreadNode)> {
        let found = self.open.store.journal_view().and_then(|(review, view)| {
            let id = self.current_note()?;
            Some((review.clone(), tree::find(&view.threads, &id)?.clone()))
        });
        if found.is_none() && !self.open.store.is_journal() {
            self.status = Some(
                "replies, resolving, sending at once and sub-threads go through the review's journal, which does not keep this document's notes"
                    .to_owned(),
            );
        }
        found
    }

    /// A reply typed at the bottom of the current note's box: only a sent thread's, where the
    /// keys offer it (`help`).
    fn begin_reply(&mut self) {
        let Some((_, node)) = self.current_thread() else { return };
        self.compose = super::super::compose::Compose::default();
        self.status = None;
        self.mode = Mode::Reply(node.id);
    }

    /// The person's reply `body` under comment `id`, a draft the next send carries, or, under a
    /// Live thread (the journal says `now`), sent at once as `send-now` sends it; an empty one
    /// is no reply.
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
        let saved = review.journal.comment(&review.target, &reply);
        self.saw(&node);
        self.status = Some(match saved {
            // The send reads the view again, the reply in it.
            Ok(saved) if saved.now => return self.send_journal_now(id),
            Ok(_) => "reply saved".to_owned(),
            Err(err) => format!("review loop: {err}"),
        });
        self.refresh_journal();
    }

    /// The person notes a passage of message `id` in its box: the thread holding it is acted
    /// on, its replies no longer new.
    pub(in crate::app) fn saw_message(&mut self, id: &str) {
        let view = self.open.store.journal_view().map(|(_, view)| view);
        if let Some(node) = view.and_then(|view| tree::holding(&view.threads, id)).cloned() {
            self.saw(&node);
        }
    }

    /// Thread `id` in the open document's view, with the review.
    fn thread_of(&self, id: &str) -> Option<(Review, ThreadNode)> {
        let (review, view) = self.open.store.journal_view()?;
        Some((review.clone(), tree::find(&view.threads, id)?.clone()))
    }

    /// Thread `id` as the review's view has it, at any depth.
    pub(super) fn node(&self, id: &str) -> Option<&ThreadNode> {
        let (_, view) = self.open.store.journal_view()?;
        tree::find(&view.threads, id)
    }

    /// The draft whose box the cursor stands on, at any depth: a paragraph of its words, its head or
    /// its folded line (its box `Not sent`), where `edit` and `confirm` edit it, as from its
    /// bubble. The innermost box: a draft sub-thread's in a sent thread, not the thread's.
    pub(in crate::app) fn draft_box(&self) -> Option<String> {
        if !self.on_box_line() {
            return None;
        }
        let id = self.thread_at_cursor()?;
        self.node(&id).is_some_and(|node| !sent(node)).then_some(id)
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

    /// Live turned on or off on the current note's thread, where the keys offer it (`help`): the
    /// view read again says it, at its head, on its folded line and in its bubble.
    fn toggle_live(&mut self) {
        let Some((review, node)) = self.current_thread() else { return };
        self.saw(&node);
        let on = node.draft || !node.live;
        // Turned on, what waits in its box goes first, at once, by the send of `send-now`
        // itself (`tree::now_drafts`): the comment while it is a draft, else the person's replies
        // waiting under it; when it did not go, the status says why and Live stays off. Turned
        // off, nothing is sent.
        let mut sent = String::new();
        if on && !tree::now_drafts(&node).is_empty() {
            self.send_journal_now(&node.id);
            if self.node(&node.id).is_none_or(|now| !tree::now_drafts(now).is_empty()) {
                return;
            }
            sent = self.status.take().map(|status| format!("{status} \u{b7} ")).unwrap_or_default();
        }
        self.status = Some(match review.journal.live(&review.target, &node.id, on) {
            Ok(()) if on => format!("{sent}live on"),
            Ok(()) => "live off".to_owned(),
            Err(err) => format!("{sent}review loop: {err}"),
        });
        self.refresh_journal();
    }

    /// The current note's draft sent alone, at once: the note itself, or the person's replies
    /// waiting under it.
    fn send_current(&mut self) {
        let Some((_, node)) = self.current_thread() else { return };
        self.saw(&node);
        self.send_journal_now(&node.id);
    }

    /// The person acted on `node`'s thread: its replies are no longer new.
    fn saw(&mut self, node: &ThreadNode) {
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.saw(node);
        }
    }
}
