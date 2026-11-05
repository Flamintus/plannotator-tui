//! Sending through the review journal: `send` (`E`) sends every draft of the review and ends
//! the round, `send-and-close` (`S`) does the same and closes, and `send-now` (`s`) sends the
//! current note's draft alone, at once, the round going on, as a box's Send does on
//! Plannotator's screens. The journal marks the drafts sent and composes what the agent reads,
//! with how to answer under each id through `plannotator reply`; that text goes to the agent's
//! pane by the app's own delivery, as any feedback does, and the journal is told it was
//! delivered. The send is archived as today's feedback is. A send the agent did not get goes
//! again first, whichever key: this pane's under its press's key, one an earlier pane left
//! pending by its id.

use std::fmt::Write as _;

use plannotator_tui_journal::{Error, IdempotencyKey, SendMode, Sent, ThreadView};

use crate::journal::Review;
use plannotator_tui_schema::{Annotation, Kind};

use super::super::App;
use super::super::draw::short_id;
use super::super::feedback::Feedback;
use super::super::send::SendState;
use crate::archive::AnnotationRecord;
use crate::delivery::DeliveryError;
use crate::journal::tree::{all_drafts, carried_by, find, now_drafts};
use crate::keys::Action;

/// A send the agent has not got yet. The journal gives the same send back, its text
/// included, rather than send twice.
#[derive(Debug)]
pub(super) enum Press {
    /// A press of this pane's, tried again under its key with what it asked: the round, or
    /// drafts at once, by id.
    Key { key: IdempotencyKey, mode: SendMode, ids: Vec<String> },
    /// A send an earlier pane left pending (the journal wrote it, the agent did not get it),
    /// by its id.
    Left(String),
}

/// The press tried again: under its key, or a left send by its id.
fn press_again(review: &Review, press: &Press) -> Result<Sent, Error> {
    match press {
        Press::Key { key, mode, ids } => {
            review.journal.send(&review.target, *mode, ids, review.to.as_ref(), key)
        }
        Press::Left(id) => review.journal.resend(&review.target, id),
    }
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

/// What a press asks the journal to send.
enum Ask {
    /// Every draft, ending the round; `named`: a round even with no draft, which names the
    /// threads still open to the agent.
    Round { named: bool },
    /// The drafts of thread `id` at once (`now_drafts`), the round going on.
    Now(String),
}

impl App {
    /// `E`: every draft of the review sent, ending the round, and the journal's text pasted
    /// into the agent's pane; a send the agent did not get yet goes again first.
    pub(in crate::app) fn send_journal(&mut self) {
        self.send_journal_round(false);
    }

    /// The same; `named`: a round even with no draft, which names the threads still open to
    /// the agent (the Review menu's resend all).
    pub(in crate::app) fn send_journal_round(&mut self, named: bool) {
        self.press_send(&Ask::Round { named });
    }

    /// `s`: thread `id`'s draft sent alone, at once, the round going on, and pasted into the
    /// agent's pane as a round is; a send the agent did not get yet goes again first.
    pub(in crate::app) fn send_journal_now(&mut self, id: &str) {
        self.press_send(&Ask::Now(id.to_owned()));
    }

    fn press_send(&mut self, ask: &Ask) {
        let Some(review) = self.loop_review() else { return };
        let retry = self.keys.hint(match ask {
            Ask::Round { .. } => Action::Send,
            Ask::Now(_) => Action::SendNow,
        });
        let press = self.review_loop.as_mut().and_then(|review_loop| review_loop.press.take());
        let (press, sent) = if let Some(press) = press {
            let sent = press_again(&review, &press);
            (press, sent)
        } else {
            // What waits now, the agent's and other panes' writes included.
            let Some(view) = self.waiting_view(&review) else { return };
            if let Some(left) = self.left_pending(&review, &view) {
                left
            } else {
                let Some((mode, ids)) = self.asked(ask, &view) else { return };
                let key = IdempotencyKey::new();
                let sent = review.journal.send(&review.target, mode, &ids, review.to.as_ref(), &key);
                (Press::Key { key, mode, ids }, sent)
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
                let round = sent.send.round;
                // A send at once leaves the round open, as the agent reads it too.
                let head = match (sent.send.mode, matches!(press, Press::Left(_))) {
                    (SendMode::Round, false) => format!("Round {round} sent"),
                    (SendMode::Round, true) => format!("Round {round} sent again"),
                    (SendMode::Now, false) => format!("Sent now \u{b7} Round {round} still open"),
                    (SendMode::Now, true) => format!("Sent again \u{b7} Round {round} still open"),
                };
                let mut status = format!("{head} \u{b7} {} comment(s) \u{2192} {target}", carried.len());
                if let Some(err) = delivered {
                    let _ = write!(status, " \u{b7} not marked delivered: {err}");
                } else {
                    // As on the screens and in the Inbox: the agent's replies come here by themselves.
                    status.push_str(" \u{b7} replies will show here");
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

    /// What `ask` sends of `view`, the review's drafts as they wait now; nothing, the footer
    /// saying so, when there is nothing to send.
    fn asked(&mut self, ask: &Ask, view: &ThreadView) -> Option<(SendMode, Vec<String>)> {
        match ask {
            Ask::Round { named: false } if all_drafts(view).is_empty() => {
                self.status = Some("nothing to send".into());
                None
            }
            Ask::Round { .. } => Some((SendMode::Round, Vec::new())),
            Ask::Now(id) => {
                let ids = find(&view.threads, id).map(now_drafts).unwrap_or_default();
                if ids.is_empty() {
                    self.status = Some(format!("nothing to send now: {} has no draft", short_id(id)));
                    return None;
                }
                Some((SendMode::Now, ids))
            }
        }
    }

    fn keep_press(&mut self, press: Press) {
        if let Some(review_loop) = self.review_loop.as_mut() {
            review_loop.press = Some(press);
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

    /// The send in the submission archive, as a feedback is: the text the agent got, and the
    /// notes it carried as they were written.
    fn archive_journal_send(&self, sent: &Sent, carried: &[String]) {
        let notes: Vec<_> = self
            .replies()
            .into_iter()
            .flat_map(|(_, open)| open.store.notes())
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

    /// The Send button over a document of the journal: the round under way and what `E` does,
    /// in the words of Plannotator's screens and Inbox: the round ends with Send Feedback.
    pub(in crate::app) fn journal_send_label(&self) -> Option<String> {
        let (_, view) = self.open.store.journal_view()?;
        let round = view.round;
        let waiting = all_drafts(view).len();
        let target = self.delivery.describe();
        let agent = self.delivery.is_agent();
        Some(match (&self.send_state, waiting, self.journal_retry_waiting()) {
            (SendState::Blocked(_), _, _) => {
                format!("Round {round} \u{b7} {target} at a dialog \u{b7} click to retry")
            }
            (_, _, true) if agent => format!("Round {round} \u{b7} Send Feedback again \u{25b8} {target}"),
            (_, _, true) => format!("Round {round} \u{b7} Copy again as feedback"),
            (_, 0, false) if agent => format!("Round {round} \u{b7} Sent \u{25b8} {target}"),
            (_, 0, false) => format!("Round {round} \u{b7} Copied"),
            (_, n, false) if agent => format!("Round {round} \u{b7} Send Feedback ({n}) \u{25b8} {target}"),
            (_, n, false) => format!("Round {round} \u{b7} Copy {n} as feedback"),
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
