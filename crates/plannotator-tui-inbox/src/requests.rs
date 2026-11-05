//! What a terminal writes. The client mints no comment id: the caller's own id becomes the
//! Inbox's (`annotation.id`, `^[A-Za-z0-9_.:-]{1,200}$`), as the window's viewer does.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A comment on words: an agent's message's, or an attached file's.
#[derive(Debug, Clone, PartialEq)]
pub struct NewComment {
    pub id: String,
    /// The words selected, as rendered (`originalText`).
    pub quote: String,
    pub text: String,
    /// More fields of the annotation, opaque to the Inbox beyond its checks (a re-anchoring
    /// hint, a file comment's `blockId` or `inboxCode`). They never override the four above.
    pub extra: Map<String, Value>,
}

impl NewComment {
    pub fn new(id: impl Into<String>, quote: impl Into<String>, text: impl Into<String>) -> Self {
        Self { id: id.into(), quote: quote.into(), text: text.into(), extra: Map::new() }
    }

    /// The annotation the Inbox stores: a comment on the words selected.
    pub(crate) fn annotation(&self) -> Value {
        let mut fields = self.extra.clone();
        fields.insert("id".to_owned(), Value::String(self.id.clone()));
        fields.insert("type".to_owned(), Value::String("COMMENT".to_owned()));
        fields.insert("originalText".to_owned(), Value::String(self.quote.clone()));
        fields.insert("text".to_owned(), Value::String(self.text.clone()));
        Value::Object(fields)
    }
}

/// The person's reply under a sent comment (a draft until a send carries it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReply {
    pub id: String,
    pub text: String,
}

/// A passage of one message of a comment thread (the comment, a reply under it, or a
/// sub-thread): its words, and which of their occurrences in that message's text, from 0.
/// The server finds where it starts; a client never counts the offset itself (the Inbox
/// counts UTF-16 units in words whose blanks are collapsed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// The message's id: a comment id, or a reply's (`rep_…` or a comment id).
    pub parent: String,
    pub quote: String,
    pub occurrence: u32,
}

/// The person's sub-thread on a passage (a draft until a send carries it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSubThread {
    pub id: String,
    pub on: Passage,
    pub text: String,
}

/// `now` sends at once and leaves the round open; `round` is the person's Finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendMode {
    Now,
    Round,
}

/// Which version of an attached file to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentVersion {
    /// The file as it is now.
    Current,
    /// The version the agent sent.
    Sent,
}

impl AttachmentVersion {
    pub(crate) fn query(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Sent => "sent",
        }
    }
}

/// A review target to deposit in the Inbox for one agent session (P1's operation, shared
/// with `/plannotator-last`): the thread for this session and target is found or made.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewTarget {
    /// `<host>:<session id>`, as `plannotator inbox review --session` names it.
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// A message of the agent's chat, by its host's id: the deposit is idempotent by
    /// session, that id and the text's sha256, which the server computes.
    ChatMessage {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        host_message_id: Option<String>,
    },
    /// A file on this machine, by path.
    File { path: String },
}

impl ReviewTarget {
    /// The agent's chat message `host_message_id` of session `session` on `host`.
    pub fn chat_message(host: &str, session: &str, host_message_id: &str, text: impl Into<String>) -> Self {
        Self {
            session: format!("{host}:{session}"),
            project_path: None,
            target: Target::ChatMessage {
                text: text.into(),
                host_message_id: Some(host_message_id.to_owned()),
            },
        }
    }
}
