//! Sending through the Inbox: `send` (`E`) sends the selected note's drafts, or every
//! draft waiting, `now`, leaving the round open; `finish-review` (`F`) finishes the round.
//! The Inbox composes what the agent reads from the comments named and wakes the session,
//! so nothing composed here is sent, and nothing goes to the agent's pane as well.

use plannotator_tui_inbox::{Error, IdempotencyKey, ReplyNode, SendMode, ThreadNode, ThreadView};

use super::super::App;
use super::super::send::SendState;
use crate::keys::Action;

/// One press of a send: tried again after the connection failed, it goes under the same
/// key, so the Inbox never sends it twice.
#[derive(Debug)]
pub(super) struct Press {
    mode: SendMode,
    ids: Vec<String>,
    key: IdempotencyKey,
}

/// The drafts of `node` and of everything under it: what one note's send carries.
fn node_drafts(node: &ThreadNode, out: &mut Vec<String>) {
    if node.draft {
        out.push(node.id.clone());
    }
    for reply in &node.replies {
        reply_drafts(reply, out);
    }
    for sub in &node.sub_threads {
        node_drafts(sub, out);
    }
}

fn reply_drafts(reply: &ReplyNode, out: &mut Vec<String>) {
    if reply.draft {
        out.push(reply.id.clone());
    }
    for sub in &reply.sub_threads {
        node_drafts(sub, out);
    }
}

/// Every draft of the thread, in the view's order.
fn drafts(view: &ThreadView) -> Vec<String> {
    let mut out = Vec::new();
    for node in &view.threads {
        node_drafts(node, &mut out);
    }
    out
}

impl App {
    /// Send through the Inbox: `now` the selected note's drafts, else every draft waiting;
    /// `round` finishes the round with whatever is left.
    pub(in crate::app) fn send_inbox(&mut self, mode: SendMode) {
        let Some((inbox, thread_id, view)) = self.open.store.inbox_thread() else { return };
        let (inbox, thread_id, round) = (inbox.clone(), thread_id.to_owned(), view.round);
        let ids = match mode {
            SendMode::Now => {
                let selected =
                    self.current_note().and_then(|id| view.threads.iter().find(|node| node.id == id));
                let mut ids = Vec::new();
                if let Some(node) = selected {
                    node_drafts(node, &mut ids);
                }
                if ids.is_empty() { drafts(view) } else { ids }
            }
            SendMode::Round => Vec::new(),
        };
        if mode == SendMode::Now && ids.is_empty() {
            self.status = Some("nothing waiting to send".into());
            return;
        }
        let key = match self.inbox.as_mut().and_then(|review| review.press.take()) {
            Some(press) if press.mode == mode && press.ids == ids => press.key,
            _ => IdempotencyKey::new(),
        };
        match inbox.send(&thread_id, mode, &ids, &key) {
            Ok(sends) => {
                let to: Vec<String> = sends.iter().map(|send| send.to.session.clone()).collect();
                let to = if to.is_empty() { String::new() } else { format!(" \u{2192} {}", to.join(", ")) };
                self.status = Some(match mode {
                    SendMode::Now => format!("sent {} now{to}", ids.len()),
                    SendMode::Round => format!("round {round} finished{to}"),
                });
            }
            Err(err) => {
                if matches!(err, Error::Transport { .. })
                    && let Some(review) = self.inbox.as_mut()
                {
                    review.press = Some(Press { mode, ids, key });
                }
                let retry =
                    self.keys.hint(if mode == SendMode::Now { Action::Send } else { Action::FinishReview });
                self.status = Some(format!("Inbox: send failed: {err} \u{b7} {retry} retry"));
            }
        }
        self.refresh_inbox();
    }

    /// The open Inbox document's drafts waiting, across its thread; `None` for any other.
    pub(in crate::app) fn inbox_drafts(&self) -> Option<usize> {
        self.open.store.inbox_thread().map(|(_, _, view)| drafts(view).len())
    }

    /// The Send button over an Inbox document: the round under way and what `E` sends.
    pub(in crate::app) fn inbox_send_label(&self) -> Option<String> {
        let (_, _, view) = self.open.store.inbox_thread()?;
        let waiting = drafts(view).len();
        let key = match self.keys.hint(Action::Send) {
            key if key.is_empty() => String::new(),
            key => format!(" ({key})"),
        };
        Some(match waiting {
            0 => format!("round {} \u{b7} nothing waiting \u{25b8} Inbox", view.round),
            n => format!("round {} \u{b7} Send {n} now \u{25b8} Inbox{key}", view.round),
        })
    }

    /// The Send button's state over an Inbox document: sent once no draft waits.
    pub(in crate::app) fn inbox_send_state(&self) -> Option<SendState> {
        let waiting = self.inbox_drafts()?;
        Some(if waiting == 0 { SendState::Sent } else { SendState::Ready })
    }

    /// Where a send goes, as the footer names it.
    pub(in crate::app) fn send_target(&self) -> String {
        if self.open.store.is_inbox() { "the Inbox".to_owned() } else { self.delivery.describe() }
    }
}
