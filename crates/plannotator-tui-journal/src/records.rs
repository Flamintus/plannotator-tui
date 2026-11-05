//! What the Inbox answers, as far as a terminal reads it. Every type keeps the fields it
//! does not name in `other`: the Inbox's answers grow additively.
//!
//! Sources (window routes behind the door): `GET health` (`InboxHealth`), `GET sessions`
//! (`InboxComposeProject`), `GET threads` (`InboxListRow` in `sections`), `GET threads/:id`
//! (`InboxThread`), `GET attachments/:id/view`, the comment routes (`InboxAnnotationRecord`),
//! comment sends (`InboxCommentSend`, E1) and the review-target deposit (P1, not built yet).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::view::Author;

/// The Inbox's health, read through the door.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub version: String,
    #[serde(rename = "serverSession")]
    pub server_session: String,
    pub pid: u64,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// Live agent sessions of one project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSessions {
    pub project: SessionProject,
    pub sessions: Vec<Session>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionProject {
    /// `None` for a folder the Inbox has not seen yet.
    pub id: Option<String>,
    pub name: String,
    pub root: String,
}

/// One live agent session: its id is its host's (Claude Code's session id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub session: String,
    pub host: String,
    pub busy: Option<bool>,
    /// The thread it last wrote in.
    pub last_thread: Option<LastThread>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastThread {
    pub id: String,
    pub subject: Option<String>,
    pub at: String,
}

/// One row of the thread list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadRow {
    pub thread_id: String,
    pub subject: Option<String>,
    pub thread_name: Option<String>,
    /// The list section it sits in (`waiting`, `new`, `quiet`, …).
    pub section: String,
    pub unread: bool,
    pub last_at: String,
    pub resolved_at: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// A thread and its messages, as the window reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub thread_id: String,
    pub subject: Option<String>,
    pub thread_name: Option<String>,
    pub resolved_at: Option<String>,
    pub messages: Vec<Message>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub reply_to: Option<String>,
    pub author: Author,
    /// Markdown, as sent.
    pub body: String,
    pub created_at: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// An attached text file's words, at one version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentText {
    /// `current`, or the sent version's sha256: what a comment on these words names.
    pub version: String,
    pub text: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// A comment as the Inbox keeps it (`InboxAnnotationRecord`): a comment, a reply under one
/// (`re`) or a sub-thread (`parent`), a draft while `sent_reply_id` is `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentRecord {
    pub id: String,
    pub thread_id: String,
    pub message_id: String,
    pub attachment_id: String,
    pub path: String,
    pub version: String,
    /// The comment as its viewer made it (`type`, `originalText`, `text`, …).
    pub annotation: Map<String, Value>,
    pub created_at: String,
    pub updated_at: String,
    pub removed_at: Option<String>,
    pub sent_reply_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_offset: Option<u64>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// One comment send: what one press sent one agent session (`InboxCommentSend`, E1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentSend {
    /// `snd_…`.
    pub id: String,
    pub thread_id: String,
    /// Shared by the sends of one press, one per session it reaches.
    pub batch: String,
    pub to: SendTarget,
    pub round: u64,
    pub mode: crate::requests::SendMode,
    /// The comments it carries, by id.
    pub comments: Vec<String>,
    pub created_at: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendTarget {
    pub host: Option<String>,
    pub session: String,
}

/// Where a deposited review target landed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deposit {
    pub thread_id: String,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    /// False when the same target was deposited before: the existing message is answered.
    pub created: bool,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}
