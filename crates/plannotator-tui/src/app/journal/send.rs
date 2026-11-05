//! Sending through the review journal: `send` (`E`) sends every draft of the review and ends
//! the round, `send-and-close` (`S`) does the same and closes. The journal marks the drafts
//! sent and composes what the agent reads, with how to answer under each id through
//! `plannotator reply`; that text goes to the agent's pane by the app's own delivery, as any
//! feedback does, and the journal is told it was delivered. The send is archived as today's
//! feedback is.

use std::fmt::Write as _;

use plannotator_tui_journal::{IdempotencyKey, SendMode, Sent, ThreadView};
use plannotator_tui_schema::{Annotation, Kind};

use super::super::App;
use super::super::feedback::Feedback;
use super::super::send::SendState;
use crate::archive::AnnotationRecord;
use crate::delivery::DeliveryError;
use crate::gate::{Decision, Gate};
use crate::journal::tree::{all_drafts, find};
use crate::keys::Action;

/// A send the agent has not got yet: the press is tried again under its key, and the journal
/// gives the same send back, its text included, rather than send twice.
#[derive(Debug)]
pub(super) struct Press {
    key: IdempotencyKey,
    /// The drafts it carried.
    carried: Vec<String>,
}

impl App {
    /// `E`: every draft of the review sent, ending the round, and the journal's text pasted
    /// into the agent's pane; a send the agent did not get yet goes again first.
    pub(in crate::app) fn send_journal(&mut self) {
        // A write in the same burst of keys has not been read back yet: its draft counts.
        if self.open.store.journal_stale() {
            self.refresh_journal();
        }
        let Some((review, view)) = self.open.store.journal_view() else { return };
        let review = review.clone();
        let press = self.review_loop.as_mut().and_then(|review_loop| review_loop.press.take());
        let press = if let Some(press) = press {
            press
        } else {
            let carried = all_drafts(view);
            if carried.is_empty() {
                self.status = Some("nothing to send".into());
                self.derive_send_state();
                return;
            }
            Press { key: IdempotencyKey::new(), carried }
        };
        let retry = self.keys.hint(Action::Send);
        let sent = match review.journal.send(&review.target, SendMode::Round, &[], &press.key) {
            Ok(sent) => sent,
            Err(err) => {
                self.status = Some(format!("send failed: {err} \u{b7} {retry} retry"));
                self.keep_press(press);
                self.refresh_journal();
                return;
            }
        };
        let target = self.delivery.describe();
        match self.delivery.deliver(&sent.text) {
            Ok(()) => {
                let mut status = format!(
                    "sent {} comment(s) \u{b7} round {} ended \u{2192} {target}",
                    press.carried.len(),
                    sent.send.round
                );
                // A copy reaches the agent when the person pastes it: the journal is not told.
                if self.delivery.is_agent()
                    && let Err(err) =
                        review.journal.delivered(&review.target, std::slice::from_ref(&sent.send.id))
                {
                    let _ = write!(status, " \u{b7} not marked delivered: {err}");
                }
                self.archive_journal_send(&sent, &press.carried);
                self.status = Some(status);
                if self.gate != Gate::Off {
                    self.decision =
                        Some(Decision::Annotated { feedback: sent.text, count: press.carried.len() });
                    self.quit = true;
                }
            }
            Err(DeliveryError::Blocked(msg)) => {
                let copied = if self.copy_fallback(&sent.text) { " \u{b7} copied to clipboard" } else { "" };
                self.status = Some(format!("{target} is at a dialog{copied} \u{b7} {retry} retry"));
                self.keep_press(press);
                self.refresh_journal();
                self.send_state = SendState::Blocked(msg);
                return;
            }
            Err(DeliveryError::Unavailable(msg)) => {
                let copied = if self.copy_fallback(&sent.text) { " \u{b7} copied to clipboard" } else { "" };
                self.status = Some(format!("no agent to send to ({msg}){copied} \u{b7} {retry} retry"));
                self.keep_press(press);
            }
            Err(DeliveryError::Failed(err)) => {
                self.status = Some(format!("send failed: {err:#} \u{b7} {retry} retry"));
                self.keep_press(press);
            }
        }
        self.refresh_journal();
    }

    fn keep_press(&mut self, press: Press) {
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.press = Some(press);
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
