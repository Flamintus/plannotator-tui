//! The computed, versioned view of a review's comment threads: the mirror of
//! `ReviewThreadView` (`packages/core/review-threads.ts`, `v: 1`), which `threads view`
//! prints.
//!
//! Plannotator computes everything a client draws: rounds, state, folds, statuses, live
//! threads, where each sub-thread sits. A client reads it and never applies those rules
//! itself. Fields the journal adds later are kept in `other`, so nothing is lost on a read.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The thread view: the round under way and the comment threads, oldest round first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadView {
    pub v: u64,
    /// The round under way (1 before anything was sent).
    pub round: u64,
    pub threads: Vec<ThreadNode>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// One thread: a comment, or a sub-thread under the message whose passage it is on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadNode {
    /// The comment's id, as its feedback heading names it.
    pub id: String,
    /// What carried it: the person's reply or a comment send (`snd_…`); empty for a draft.
    pub reply_id: String,
    pub round: u64,
    pub state: State,
    /// Where it is (flat on the wire).
    #[serde(flatten)]
    pub place: Place,
    pub text: String,
    /// The person's draft, not sent yet.
    pub draft: bool,
    /// Drawn folded: resolved, with nothing open under it at any depth.
    pub folded: bool,
    /// What became of it, once a comment send carried it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    /// A `now` send reached it: the person's next replies in it go on their own.
    pub live: bool,
    pub replies: Vec<ReplyNode>,
    /// The sub-threads on its own words.
    pub sub_threads: Vec<ThreadNode>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// Where a thread is: its anchor, the words it is on, whether they are still found, and,
/// for a sub-thread, the message whose passage it is on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub on: Anchor,
    /// The words, lines or part it is on, as the person selected them.
    pub quote: String,
    /// Its words were not found in the target the last time a client showed it.
    pub unanchored: bool,
    /// A sub-thread: the message it is on and where its passage starts in those words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Parent>,
}

/// One reply under a thread, with the sub-threads on its words.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyNode {
    /// `rep_…` for an agent's; the person's is a comment id.
    pub id: String,
    pub author: Author,
    pub round: u64,
    /// What it came with: the person's send, or the agent's message; empty for a draft.
    pub message_id: String,
    pub text: String,
    pub at: String,
    /// The agent's name as people read it; `None` for the person's, whom a client names.
    pub author_name: Option<String>,
    pub draft: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    pub sub_threads: Vec<ThreadNode>,
    /// Replies to this reply, for a source whose replies nest. The journal never fills it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replies: Option<Vec<ReplyNode>>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Open,
    Resolved,
}

/// What became of an element a comment send carried, as the person sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Delivered,
    Read,
    Working,
    Replied,
    /// A status this client does not know yet.
    #[serde(other)]
    Unknown,
}

/// Where a comment is, in its review's target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    /// A passage of an agent's message, by the id the client names it with.
    Message { message_id: String },
    /// A file's words, and the lines they are on when known.
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<[u64; 2]>,
    },
    /// The plan under review: the version commented and the lines, when known.
    Plan {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<[u64; 2]>,
    },
    /// A place this client does not know yet.
    #[serde(other)]
    Unknown,
}

/// The message a sub-thread is on, and where its passage starts in its words (UTF-16 units,
/// whitespace runs counted as one: `passageOffset` in Plannotator's `passage.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent {
    pub id: String,
    pub quote_offset: u64,
}

/// Who wrote a message or a reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Author {
    Person,
    Agent { host: Option<String>, session: Option<String>, name: Option<String> },
}
