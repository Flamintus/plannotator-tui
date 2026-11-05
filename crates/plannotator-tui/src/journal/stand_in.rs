//! A stand-in for the review journal, for the app's tests: the person's comments, the
//! agent's replies and the sends held in memory, listed by a view of the journal's shape
//! (`comments.rs`). A call a test lists in `refused` is refused, as the CLI refuses it.

#![allow(clippy::expect_used, reason = "test fixtures assert by panicking")]

mod comments;

use std::cell::RefCell;

use plannotator_tui_journal::{
    Error, IdempotencyKey, Journal, NewComment, Placement, ReviewTarget, Send, SendMode, SendTo, Sent, Stamp,
    Status, ThreadView, Viewed,
};

pub(crate) use comments::Comment;
use comments::view;

/// The key the stand-in names its one review by.
pub(crate) const REVIEW: &str = "last-0123456789abcdef01234567";

#[derive(Debug, Default)]
pub(crate) struct StandIn {
    pub(crate) state: RefCell<State>,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) comments: Vec<Comment>,
    /// Every write, in order: `comment <id>`, `remove <id>`, `resolve <id> <bool>`.
    pub(crate) writes: Vec<String>,
    /// Rounds finished: the round under way is the next one.
    pub(crate) finished: u64,
    /// Every send written: its mode, the comments it carried and its key.
    pub(crate) sends: Vec<(SendMode, Vec<String>, String)>,
    /// The session each send named, in the order written (`None`: the review's own).
    pub(crate) sent_to: Vec<Option<SendTo>>,
    /// The sends given back by their id, in order.
    pub(crate) resent: Vec<String>,
    /// The review was closed (approved): its journal went.
    pub(crate) closed: bool,
    /// The sends marked delivered, by id.
    pub(crate) delivered: Vec<String>,
    /// Calls refused, by action: `view`, `comment`, `remove`, `resolve`, `send`, `resend`,
    /// `delivered`, `close`.
    pub(crate) refused: Vec<&'static str>,
    /// The code they are refused with: `locked` unless a test names another.
    pub(crate) refused_as: Option<&'static str>,
    /// A view to answer as it is, in place of the one the comments make: one computed by
    /// Plannotator's own code (the client crate's fixture).
    pub(crate) view: Option<ThreadView>,
    /// The review each call named.
    pub(crate) targets: Vec<ReviewTarget>,
    /// Bumped by every write: what `stamp` says.
    pub(crate) written: u64,
}

impl State {
    /// Send `index` (from 0) as the journal answers it: its round, what it carried, where it
    /// went and the text for the agent.
    fn sent(&self, index: usize, replayed: bool) -> Sent {
        let (mode, carried, _) = self.sends.get(index).cloned().expect("a send");
        let before = self.sends.iter().take(index).filter(|(mode, _, _)| *mode == SendMode::Round).count();
        let round = before as u64 + 1;
        let to = self.sent_to.get(index).cloned().flatten();
        let to = to.unwrap_or_else(|| SendTo { host: Some("claude-code".into()), session: "ses_a".into() });
        Sent {
            review: REVIEW.to_owned(),
            text: StandIn::text(round, &carried),
            send: Send {
                id: format!("snd_{}", index + 1),
                round,
                mode,
                comments: carried,
                to,
                other: serde_json::Map::new(),
            },
            replayed,
        }
    }

    /// The agent replied `text` under comment `id` (another process wrote the journal).
    pub(crate) fn agent_replies(&mut self, reply_id: &str, under: &str, text: &str) {
        let comment = self.comments.iter().find(|comment| comment.id == under).expect("the comment").clone();
        self.comments.push(Comment::agent_reply(reply_id, &comment, text));
        if let Some(comment) = self.comments.iter_mut().find(|comment| comment.id == under) {
            comment.status = Some(Status::Replied);
        }
        self.written += 1;
    }
}

fn refused(action: &str, code: &str) -> Error {
    Error::Refused { action: action.to_owned(), code: code.to_owned(), message: code.replace('_', " ") }
}

impl StandIn {
    /// Refuse `action` when a test asked for it; note the review named.
    fn call(&self, action: &'static str, target: &ReviewTarget) -> Result<(), Error> {
        let mut state = self.state.borrow_mut();
        state.targets.push(target.clone());
        if state.refused.contains(&action) {
            return Err(refused(action, state.refused_as.unwrap_or("locked")));
        }
        Ok(())
    }

    /// The text a send hands the agent: its round and what it carries.
    fn text(round: u64, carried: &[String]) -> String {
        format!("# Comments on your work\n\nround {round}: {}", carried.join(", "))
    }
}

impl Journal for StandIn {
    fn view(&self, target: &ReviewTarget) -> Result<Viewed, Error> {
        self.call("view", target)?;
        let state = self.state.borrow();
        let view = state.view.clone().unwrap_or_else(|| ThreadView {
            v: 1,
            round: state.finished + 1,
            threads: view(&state.comments),
            other: serde_json::Map::new(),
        });
        Ok(Viewed { review: REVIEW.to_owned(), view })
    }

    /// A draft written as the journal writes one: the same id again replaces the whole
    /// draft, its place and kind with it; a sent one is not edited.
    fn comment(&self, target: &ReviewTarget, comment: &NewComment) -> Result<(), Error> {
        self.call("comment", target)?;
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("comment {}", comment.id));
        let known = state.comments.iter().position(|kept| kept.id == comment.id);
        if known.and_then(|at| state.comments.get(at)).is_some_and(|kept| !kept.draft) {
            return Err(refused("comment", "conflict"));
        }
        let sent = |state: &State, id: &str| state.comments.iter().find(|c| c.id == id && !c.draft).cloned();
        let kept = match &comment.placement {
            Placement::On { on, quote, deletion } => {
                let mut kept = Comment::draft(&comment.id, on.clone(), quote, &comment.text);
                kept.deletion = *deletion;
                kept
            }
            Placement::Under { re } => {
                let under = sent(&state, re).ok_or_else(|| refused("comment", "not_found"))?;
                let mut kept = Comment::draft(&comment.id, under.on, "", &comment.text);
                kept.re = Some(re.clone());
                kept
            }
            Placement::Passage { parent, quote, quote_offset } => {
                let on = sent(&state, parent).ok_or_else(|| refused("comment", "not_found"))?.on;
                let mut kept = Comment::draft(&comment.id, on, quote, &comment.text);
                kept.parent = Some(parent.clone());
                kept.quote_offset = *quote_offset;
                kept
            }
        };
        match known.and_then(|at| state.comments.get_mut(at)) {
            Some(slot) => *slot = kept,
            None => state.comments.push(kept),
        }
        state.written += 1;
        Ok(())
    }

    /// Removes a draft and everything of the person's under it, at any depth.
    fn remove(&self, target: &ReviewTarget, id: &str) -> Result<(), Error> {
        self.call("remove", target)?;
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("remove {id}"));
        if !state.comments.iter().any(|comment| comment.id == id) {
            return Err(refused("remove", "not_found"));
        }
        let mut gone = vec![id.to_owned()];
        loop {
            let under: Vec<String> = state
                .comments
                .iter()
                .filter(|c| !c.agent && !gone.contains(&c.id))
                .filter(|c| {
                    let up = c.re.as_ref().or(c.parent.as_ref());
                    up.is_some_and(|up| {
                        gone.contains(up)
                            || state.comments.iter().any(|r| {
                                r.agent && &r.id == up && r.re.as_ref().is_some_and(|re| gone.contains(re))
                            })
                    })
                })
                .map(|c| c.id.clone())
                .collect();
            if under.is_empty() {
                break;
            }
            gone.extend(under);
        }
        state.comments.retain(|comment| comment.agent || !gone.contains(&comment.id));
        state.written += 1;
        Ok(())
    }

    fn resolve(&self, target: &ReviewTarget, id: &str, resolved: bool) -> Result<(), Error> {
        self.call("resolve", target)?;
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("resolve {id} {resolved}"));
        let Some(kept) = state.comments.iter_mut().find(|comment| comment.id == id && !comment.draft) else {
            return Err(refused("resolve", "not_found"));
        };
        kept.resolved = resolved;
        state.written += 1;
        Ok(())
    }

    /// `round` sends every draft left and finishes the round, and is refused (`empty`) with
    /// none; the same key again gives the first send back.
    fn send(
        &self,
        target: &ReviewTarget,
        mode: SendMode,
        ids: &[String],
        to: Option<&SendTo>,
        key: &IdempotencyKey,
    ) -> Result<Sent, Error> {
        self.call("send", target)?;
        let mut state = self.state.borrow_mut();
        if let Some(index) = state.sends.iter().position(|(_, _, kept)| kept == key.as_str()) {
            return Ok(state.sent(index, true));
        }
        let send_id = format!("snd_{}", state.sends.len() + 1);
        let mut carried = Vec::new();
        for comment in
            state.comments.iter_mut().filter(|c| c.draft && (mode == SendMode::Round || ids.contains(&c.id)))
        {
            comment.draft = false;
            comment.status = Some(Status::Pending);
            comment.send = Some(send_id.clone());
            carried.push(comment.id.clone());
        }
        if mode == SendMode::Round && carried.is_empty() {
            return Err(refused("send", "empty"));
        }
        if mode == SendMode::Round {
            state.finished += 1;
        }
        state.sends.push((mode, carried, key.as_str().to_owned()));
        state.sent_to.push(to.cloned());
        state.written += 1;
        Ok(state.sent(state.sends.len() - 1, false))
    }

    /// Send `snd_<n>` given back, its text composed again; nothing written.
    fn resend(&self, target: &ReviewTarget, id: &str) -> Result<Sent, Error> {
        self.call("resend", target)?;
        let mut state = self.state.borrow_mut();
        state.resent.push(id.to_owned());
        let index =
            id.strip_prefix("snd_").and_then(|n| n.parse::<usize>().ok()).and_then(|n| n.checked_sub(1));
        let index =
            index.filter(|index| *index < state.sends.len()).ok_or_else(|| refused("send", "not_found"))?;
        Ok(state.sent(index, true))
    }

    fn delivered(&self, target: &ReviewTarget, sends: &[String]) -> Result<(), Error> {
        self.call("delivered", target)?;
        let mut state = self.state.borrow_mut();
        state.delivered.extend(sends.iter().cloned());
        for comment in state.comments.iter_mut().filter(|c| c.status == Some(Status::Pending)) {
            comment.status = Some(Status::Delivered);
        }
        state.written += 1;
        Ok(())
    }

    fn close(&self, target: &ReviewTarget) -> Result<(), Error> {
        self.call("close", target)?;
        let mut state = self.state.borrow_mut();
        state.closed = true;
        state.comments.clear();
        state.written += 1;
        Ok(())
    }

    fn stamp(&self, review: &str) -> Option<Stamp> {
        (review == REVIEW).then(|| Stamp::new(self.state.borrow().written, None))
    }
}
