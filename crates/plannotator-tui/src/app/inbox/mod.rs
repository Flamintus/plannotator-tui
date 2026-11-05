//! Reviewing through the Plannotator Inbox (`crate::inbox`): an Inbox thread's agent
//! messages and attached files in the picker, each opened as a document.

use anyhow::Result;

use super::{App, Mode, Open};
use crate::delivery::Delivery;
use crate::inbox::source::{Document, ThreadDocuments};

/// What an Inbox review keeps besides its documents.
pub(super) struct InboxReview {
    /// What each picker candidate opens, by candidate index, when the candidates are an
    /// Inbox thread's documents.
    documents: Vec<Document>,
}

impl std::fmt::Debug for InboxReview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboxReview").field("documents", &self.documents.len()).finish_non_exhaustive()
    }
}

impl App {
    /// Review an Inbox thread: its newest document open, the picker offering them all when
    /// there is more than one.
    pub(crate) fn open_inbox_thread(
        found: ThreadDocuments,
        width: usize,
        delivery: Box<dyn Delivery>,
    ) -> Result<Self> {
        let Some(first) = found.documents.first() else {
            anyhow::bail!("Inbox thread \"{}\" has no agent message to review", found.subject)
        };
        let mut app = Self::open(first.source(), width, delivery)?;
        app.candidates = found.documents.iter().map(Document::candidate).collect();
        "Inbox".clone_into(&mut app.message_host);
        if app.candidates.len() > 1 {
            app.mode = Mode::Pick;
        }
        if !found.unreadable.is_empty() {
            app.status = Some(format!("could not read {}", found.unreadable.join("; ")));
        }
        app.inbox = Some(InboxReview { documents: found.documents });
        Ok(app)
    }

    /// The document picker candidate `index` opens, when the candidates are an Inbox
    /// thread's; `None` for an agent's replies.
    pub(super) fn inbox_candidate(&self, index: usize) -> Result<Option<Open>> {
        let Some(document) = self.inbox.as_ref().and_then(|review| review.documents.get(index)) else {
            return Ok(None);
        };
        Open::new(document.source(), self.open.layout.width, &self.data_dir, &self.project).map(Some)
    }
}

#[cfg(test)]
mod tests;
