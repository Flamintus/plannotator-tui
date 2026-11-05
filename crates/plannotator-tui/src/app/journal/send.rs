//! Sending through the review journal: `send` (`E`) sends every draft of the review and ends
//! the round, `send-and-close` (`S`) does the same and closes. The journal marks the drafts
//! sent and composes what the agent reads, with how to answer under each id through
//! `plannotator reply`; that text goes to the agent's pane by the app's own delivery, as any
//! feedback does, and the journal is told it was delivered. The send is archived as today's
//! feedback is. A send the agent did not get goes again first: this pane's under its press's
//! key, one an earlier pane left pending by its id.

use std::fmt::Write as _;

use plannotator_tui_journal::{Error, IdempotencyKey, SendMode, Sent, ThreadView};

use crate::journal::Review;
use plannotator_tui_schema::{Annotation, Kind};

use super::super::App;
use super::super::feedback::Feedback;
use super::super::send::SendState;
use crate::archive::AnnotationRecord;
use crate::delivery::DeliveryError;
use crate::journal::tree::{all_drafts, carried_by, find, pending_sends};
use crate::keys::Action;

/// A send the agent has not got yet. The journal gives the same send back, its text
/// included, rather than send twice.
#[derive(Debug)]
pub(super) enum Press {
    /// A press of this pane's, tried again under its key.
    Key(IdempotencyKey),
    /// A send an earlier pane left pending (the journal wrote it, the agent did not get it),
    /// by its id.
    Left(String),
}

/// Whether a refused send may go through when tried again: the CLI did not run or answer,
/// or the journal was locked for a moment. Any other refusal would only repeat.
fn passing(err: &Error) -> bool {
    match err {
        Error::Spawn { .. } | Error::Timeout { .. } | Error::Failed { .. } => true,
        Error::Refused { code, .. } => code == "locked",
        _ => false,
    }
}

impl App {
    /// `E`: every draft of the review sent, ending the round, and the journal's text pasted
    /// into the agent's pane; a send the agent did not get yet goes again first.
    pub(in crate::app) fn send_journal(&mut self) {
        let Some(review) = self.loop_review() else { return };
        let retry = self.keys.hint(Action::Send);
        let press = self.review_loop.as_mut().and_then(|review_loop| review_loop.press.take());
        let (press, sent) = if let Some(press) = press {
            let sent = Self::press_again(&review, &press);
            (press, sent)
        } else {
            // What waits now, the agent's and other panes' writes included.
            let Some(view) = self.waiting_view(&review) else { return };
            if let Some(left) = self.left_pending(&review, &view) {
                left
            } else if all_drafts(&view).is_empty() {
                self.status = Some("nothing to send".into());
                return;
            } else {
                let key = IdempotencyKey::new();
                let sent =
                    review.journal.send(&review.target, SendMode::Round, &[], review.to.as_ref(), &key);
                (Press::Key(key), sent)
            }
        };
        let sent = match sent {
            Ok(sent) => sent,
            Err(err) if err.code() == Some("empty") => {
                self.refresh_journal();
                self.status = Some("nothing to send".into());
                return;
            }
            Err(err) => {
                if passing(&err) {
                    self.keep_press(press);
                }
                self.refresh_journal();
                self.status = Some(format!("send failed: {err} \u{b7} {retry} retry"));
                return;
            }
        };
        let target = self.delivery.describe();
        match self.delivery.deliver(&sent.text) {
            Ok(()) => {
                // A copy reaches the agent when the person pastes it: the journal is not told.
                let delivered = if self.delivery.is_agent() {
                    review.journal.delivered(&review.target, std::slice::from_ref(&sent.send.id)).err()
                } else {
                    None
                };
                self.refresh_journal();
                let carried = self
                    .open
                    .store
                    .journal_view()
                    .map(|(_, view)| carried_by(view, &sent.send.id))
                    .unwrap_or_default();
                self.archive_journal_send(&sent, &carried);
                let again = if matches!(press, Press::Left(_)) { " again" } else { "" };
                let mut status = format!(
                    "sent{again} {} comment(s) \u{b7} round {} ended \u{2192} {target}",
                    carried.len(),
                    sent.send.round
                );
                if let Some(err) = delivered {
                    let _ = write!(status, " \u{b7} not marked delivered: {err}");
                }
                self.status = Some(status);
            }
            Err(error) => {
                // The journal wrote the send: it waits for the next press, under the same key.
                let copied = match &error {
                    DeliveryError::Failed(_) => false,
                    DeliveryError::Blocked(_) | DeliveryError::Unavailable(_) => {
                        self.copy_fallback(&sent.text)
                    }
                };
                let copied = if copied { " \u{b7} copied to clipboard" } else { "" };
                self.keep_press(press);
                self.refresh_journal();
                self.status = Some(match &error {
                    DeliveryError::Blocked(_) => {
                        format!("{target} is at a dialog{copied} \u{b7} {retry} retry")
                    }
                    DeliveryError::Unavailable(msg) => {
                        format!("no agent to send to ({msg}){copied} \u{b7} {retry} retry")
                    }
                    DeliveryError::Failed(err) => format!("send failed: {err:#} \u{b7} {retry} retry"),
                });
                if let DeliveryError::Blocked(msg) = error {
                    self.send_state = SendState::Blocked(msg);
                }
            }
        }
    }

    fn keep_press(&mut self, press: Press) {
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.press = Some(press);
        }
    }

    /// The press tried again: under its key, or a left send by its id.
    fn press_again(review: &Review, press: &Press) -> Result<Sent, Error> {
        match press {
            Press::Key(key) => {
                review.journal.send(&review.target, SendMode::Round, &[], review.to.as_ref(), key)
            }
            Press::Left(id) => review.journal.resend(&review.target, id),
        }
    }

    /// The review `E` sends: the open document's, or the file's or folder's under way.
    fn loop_review(&self) -> Option<Review> {
        let open = self.open.store.journal_view().map(|(review, _)| review);
        open.or_else(|| self.path_review()).cloned()
    }

    /// The review's view as it is now, read again; the footer says why when it cannot be.
    fn waiting_view(&mut self, review: &Review) -> Option<ThreadView> {
        self.refresh_journal();
        if let Some((_, view)) = self.open.store.journal_view() {
            return Some(view.clone());
        }
        match review.journal.view(&review.target) {
            Ok(viewed) => Some(viewed.view),
            Err(err) => {
                self.status = Some(format!("review loop: {err}"));
                None
            }
        }
    }

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
            let send = self.keys.hint(Action::Send);
            self.add_status(&format!(
                "review loop: {drafts} draft(s) left by an earlier pane go with the next {send}"
            ));
        }
    }

    /// The next send an earlier pane left that is still pending in `view` and went to this
    /// review's session, given back by the journal with its text; the others are dropped.
    fn left_pending(&mut self, review: &Review, view: &ThreadView) -> Option<(Press, Result<Sent, Error>)> {
        let pending = pending_sends(view);
        loop {
            let id = self.review_loop.as_mut()?.left.pop()?;
            if !pending.contains(&id) {
                continue;
            }
            let sent = review.journal.resend(&review.target, &id);
            // A send to another session (a screen's) is not this pane's to hand over.
            if let Ok(found) = &sent
                && review.session().is_some_and(|session| session != found.send.to.session)
            {
                continue;
            }
            return Some((Press::Left(id), sent));
        }
    }

    /// The send in the submission archive, as a feedback is: the text the agent got, and the
    /// notes it carried as they were written.
    fn archive_journal_send(&self, sent: &Sent, carried: &[String]) {
        let notes: Vec<_> = self
            .replies()
            .into_iter()
            .flat_map(|(_, open)| open.store.placed())
            .filter(|placed| carried.contains(&placed.annotation.id))
            .map(|placed| placed.annotation.clone())
            .collect();
        let view = self.open.store.journal_view().map(|(_, view)| view);
        let annotations = carried.iter().map(|id| record(id, &notes, view)).collect();
        let mut feedback = Feedback {
            text: sent.text.clone(),
            count: carried.len(),
            images: notes.iter().map(|note| note.plannotator_tui.images.len()).sum(),
            annotations,
            ..Feedback::default()
        };
        self.archive_submission(&mut feedback, "feedback");
    }

    /// The drafts of the review waiting, when the open document is the journal's.
    pub(in crate::app) fn journal_drafts(&self) -> Option<usize> {
        self.open.store.journal_view().map(|(_, view)| all_drafts(view).len())
    }

    /// Whether a send the agent did not get waits to be tried again.
    pub(in crate::app) fn journal_retry_waiting(&self) -> bool {
        self.open.store.is_journal()
            && self.review_loop.as_ref().is_some_and(|review_loop| review_loop.press.is_some())
    }

    /// The Send button over a document of the journal: the round under way and what `E` does.
    pub(in crate::app) fn journal_send_label(&self) -> Option<String> {
        let (_, view) = self.open.store.journal_view()?;
        let round = view.round;
        let waiting = all_drafts(view).len();
        let target = self.delivery.describe();
        let agent = self.delivery.is_agent();
        Some(match (&self.send_state, waiting, self.journal_retry_waiting()) {
            (SendState::Blocked(_), _, _) => {
                format!("round {round} \u{b7} {target} at a dialog \u{b7} click to retry")
            }
            (_, _, true) if agent => format!("round {round} \u{b7} Send again to {target} \u{25b8}"),
            (_, _, true) => format!("round {round} \u{b7} Copy again as feedback"),
            (_, 0, false) if agent => format!("round {round} \u{b7} Sent \u{25b8} {target}"),
            (_, 0, false) => format!("round {round} \u{b7} Copied"),
            (_, n, false) if agent => format!("round {round} \u{b7} Send {n} to {target} \u{25b8}"),
            (_, n, false) => format!("round {round} \u{b7} Copy {n} as feedback"),
        })
    }

    /// The Send button's state over a document of the journal: sent once nothing waits.
    pub(in crate::app) fn journal_send_state(&self) -> Option<SendState> {
        let waiting = self.journal_drafts()?;
        Some(if waiting == 0 && !self.journal_retry_waiting() { SendState::Sent } else { SendState::Ready })
    }
}

/// The archive's record of comment `id` a send carried: its note as written, or, for a reply
/// or a sub-thread, its words as the view has them.
fn record(id: &str, notes: &[Annotation], view: Option<&ThreadView>) -> AnnotationRecord {
    if let Some(note) = notes.iter().find(|note| note.id == id) {
        let kind = match note.anchor.kind() {
            Kind::Comment => "comment",
            Kind::LooksGood => "looks-good",
            Kind::Delete => "delete",
        };
        return AnnotationRecord {
            id: Some(id.to_owned()),
            kind: Some(kind.to_owned()),
            text: (!note.body.is_empty()).then(|| note.body.clone()),
            original_text: (!note.anchor.original_text.is_empty()).then(|| note.anchor.original_text.clone()),
        };
    }
    let node = view.and_then(|view| find(&view.threads, id));
    AnnotationRecord {
        id: Some(id.to_owned()),
        kind: Some("comment".to_owned()),
        text: node.map(|node| node.text.clone()),
        original_text: node.map(|node| node.place.quote.clone()).filter(|quote| !quote.is_empty()),
    }
}
