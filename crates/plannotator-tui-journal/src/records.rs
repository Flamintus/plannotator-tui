//! What the CLI answers, as far as a terminal reads it. Every type keeps the fields it does
//! not name in `other`: the journal's answers grow additively.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::requests::SendMode;
use crate::view::ThreadView;

/// `threads view`: the review's key, and its thread view, drafts included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Viewed {
    /// The review's key (`last-…`): the one the text sent to the agent names, and its
    /// journal's folder.
    pub review: String,
    pub view: ThreadView,
}

/// `threads send`: the send the journal wrote (or gave back, `replayed`), and the text that
/// hands it to the agent, composed by Plannotator with how to answer through its CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sent {
    pub review: String,
    pub send: Send,
    pub replayed: bool,
    pub text: String,
}

/// One send: what one press sends the agent's session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Send {
    /// `snd_…`.
    pub id: String,
    /// The round it was sent in.
    pub round: u64,
    pub mode: SendMode,
    /// What it carries, by id: the drafts it sent and, at the end of a round, the threads
    /// still open that went before, named only.
    pub comments: Vec<String>,
    pub to: SendTo,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// The agent session a send goes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendTo {
    pub host: Option<String>,
    pub session: String,
}
