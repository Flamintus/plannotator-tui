//! An Inbox thread's documents: each agent message, and each text file attached to one,
//! newest message first, its files after it. Only an agent's words take comments in the
//! Inbox, so the person's own messages are not offered.

use anyhow::{Context, Result};
use plannotator_tui_hosts::{Message, Role};
use plannotator_tui_journal::{AttachmentVersion, Author, Inbox};
use serde::Deserialize;

use super::Target;

/// Attachment kinds whose words a terminal shows as they are.
const TEXT_KINDS: [&str; 6] = ["markdown", "text", "code", "mermaid", "graphviz", "html"];

/// One document of a thread: what it is, what to call it, its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Document {
    pub(crate) target: Target,
    pub(crate) name: String,
    pub(crate) text: String,
    /// When its message was sent: the picker's clock.
    pub(crate) at: String,
}

impl Document {
    /// Its line in the picker: a message by its words, a file by its name.
    pub(crate) fn candidate(&self) -> Message {
        let (id, text) = match &self.target {
            Target::Message { message_id, .. } => (message_id.clone(), self.text.clone()),
            Target::Thread { comment_id, .. } => (comment_id.clone(), self.text.clone()),
            Target::Attachment { attachment_id, path, .. } => {
                let name = std::path::Path::new(path).file_name().and_then(|name| name.to_str());
                (attachment_id.clone(), format!("file {}", name.unwrap_or(path)))
            }
        };
        Message { id, role: Role::Assistant, text, at: Some(self.at.clone()) }
    }
}

/// A file attached to a message, as far as this app reads it (`InboxAttachment`).
#[derive(Debug, Deserialize)]
struct Attached {
    id: String,
    path: String,
    name: String,
    kind: String,
}

/// What a thread offers to review, and the attached files that could not be read.
#[derive(Debug, Default)]
pub(crate) struct ThreadDocuments {
    pub(crate) subject: String,
    pub(crate) documents: Vec<Document>,
    pub(crate) unreadable: Vec<String>,
}

/// Thread `thread_id`'s documents. `sent`: its files as the agent sent them, rather than
/// as they are now.
pub(crate) fn thread_documents(inbox: &dyn Inbox, thread_id: &str, sent: bool) -> Result<ThreadDocuments> {
    let thread = inbox.thread(thread_id).with_context(|| format!("reading Inbox thread {thread_id}"))?;
    let subject = thread.thread_name.or(thread.subject).unwrap_or_else(|| thread_id.to_owned());
    let version = if sent { AttachmentVersion::Sent } else { AttachmentVersion::Current };
    let mut groups = Vec::new();
    let mut unreadable = Vec::new();
    for message in thread.messages.iter().filter(|m| matches!(m.author, Author::Agent { .. })) {
        let mut group = vec![Document {
            target: Target::Message { thread_id: thread_id.to_owned(), message_id: message.id.clone() },
            name: format!("{subject} \u{b7} message"),
            text: message.body.clone(),
            at: message.created_at.clone(),
        }];
        let files: Vec<Attached> = message
            .other
            .get("attachments")
            .and_then(|files| serde_json::from_value(files.clone()).ok())
            .unwrap_or_default();
        for file in files.into_iter().filter(|file| TEXT_KINDS.contains(&file.kind.as_str())) {
            match inbox.attachment_text(&file.id, version) {
                Ok(words) => group.push(Document {
                    target: Target::Attachment {
                        thread_id: thread_id.to_owned(),
                        message_id: message.id.clone(),
                        attachment_id: file.id,
                        path: file.path,
                        version: words.version,
                        sent,
                    },
                    name: format!("{} \u{b7} {subject}", file.name),
                    text: words.text,
                    at: message.created_at.clone(),
                }),
                Err(err) => unreadable.push(format!("{}: {err}", file.name)),
            }
        }
        groups.push(group);
    }
    let documents = groups.into_iter().rev().flatten().collect();
    Ok(ThreadDocuments { subject, documents, unreadable })
}
