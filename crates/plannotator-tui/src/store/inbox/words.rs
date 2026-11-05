//! Where an Inbox comment is in its document: which comments of the thread's view are on
//! the document's words, where each one's quote is in them, and the passage a new
//! sub-thread is on in a comment's thread opened as a document.

use std::collections::BTreeMap;
use std::ops::Range;

use anyhow::{Context, Result};
use plannotator_tui_inbox::{
    Anchor as On, FileVersion, NewComment, NewSubThread, Passage, State as ThreadState, ThreadNode,
    ThreadView,
};
use plannotator_tui_schema::{Anchor, Annotation, Kind, NoteExtras, Reply, SourceRange, State, blob_sha};

use super::Kept;
use crate::doc::Document;
use crate::inbox::{Target, passage, tree};

impl Kept {
    /// Write `annotation` as a new comment, or as an edit of its draft.
    pub(super) fn comment(&self, annotation: &Annotation) -> Result<()> {
        let mut comment =
            NewComment::new(&annotation.id, annotation.anchor.rendered(), Self::text(annotation));
        // Which occurrence of its words it is on, for a reader that places it again later: the
        // Inbox keeps the field; its view does not hand it back yet (E7).
        if let Some(start) =
            annotation.anchor.plannotator_tui.as_ref().and_then(|e| e.source.as_ref()).map(|r| r.start)
        {
            let occurrence = passage::occurrence(&self.chars, &comment.quote, start);
            comment
                .extra
                .insert("plannotator_tui".to_owned(), serde_json::json!({ "occurrence": occurrence }));
        }
        match &self.target {
            Target::Message { message_id, .. } => self.inbox.comment_on_message(message_id, &comment),
            Target::Attachment { attachment_id, version, .. } => {
                self.inbox.comment_on_attachment(attachment_id, version, &comment)
            }
            Target::Thread { .. } => {
                let sub = NewSubThread {
                    id: comment.id.clone(),
                    on: self.passage(annotation)?,
                    text: comment.text,
                };
                self.inbox.sub_thread(&sub)
            }
        }
        .map(drop)
        .with_context(|| format!("comment {}", annotation.id))
    }

    /// The comments of `view` on this document's own words: on a message or a file, its
    /// comments; on a comment's thread, the sub-threads on its messages.
    pub(super) fn comments<'v>(&self, view: &'v ThreadView) -> Vec<&'v ThreadNode> {
        let Target::Thread { comment_id, .. } = &self.target else {
            return view.threads.iter().filter(|node| self.is_on_this(node)).collect();
        };
        let Some(thread) = tree::find(&view.threads, comment_id) else { return Vec::new() };
        let replies = thread.replies.iter().flat_map(|reply| &reply.sub_threads);
        thread.sub_threads.iter().chain(replies).collect()
    }

    /// The source range of message `id`'s words, in a comment's thread opened as a document.
    fn message(&self, id: &str) -> Option<Range<usize>> {
        let Target::Thread { messages, .. } = &self.target else { return None };
        messages.iter().find(|(message, _)| message == id).map(|(_, range)| range.clone())
    }

    /// The document's rendered characters within `range`.
    fn chars_in(&self, range: &Range<usize>) -> Vec<(char, usize)> {
        self.chars.iter().filter(|(_, offset)| range.contains(offset)).copied().collect()
    }

    /// The passage a new sub-thread is on: the message whose words hold the selection, its
    /// quote, and which of the quote's occurrences there it is.
    fn passage(&self, annotation: &Annotation) -> Result<Passage> {
        let Target::Thread { messages, .. } = &self.target else { anyhow::bail!("not a comment's thread") };
        let source = annotation.anchor.plannotator_tui.as_ref().and_then(|extras| extras.source.as_ref());
        let (start, end) = source.map(|range| (range.start, range.end)).context("no selection to place")?;
        let (parent, range) = messages
            .iter()
            .find(|(_, range)| range.start <= start && end <= range.end)
            .context("a sub-thread is on one message's words: select within one")?;
        let quote = annotation.anchor.rendered().to_owned();
        let occurrence = passage::occurrence(&self.chars_in(range), &quote, start);
        Ok(Passage {
            parent: parent.clone(),
            quote,
            occurrence: u32::try_from(occurrence).unwrap_or(u32::MAX),
        })
    }

    /// Whether `node` is a comment on this document's own words.
    fn is_on_this(&self, node: &ThreadNode) -> bool {
        node.place.parent.is_none()
            && match (&node.place.on, &self.target) {
                (On::Message { message_id }, Target::Message { message_id: this, .. }) => message_id == this,
                (On::File { path, version, .. }, Target::Attachment { path: this, sent, .. }) => {
                    path == this && (*version == FileVersion::Sent) == *sent
                }
                _ => false,
            }
    }

    /// Where `node`'s quote is in the document: where this review put it, else the first
    /// place the words as drawn hold it (within its message's, for a sub-thread), else an
    /// anchor that resolves to nothing.
    pub(super) fn anchor(&self, node: &ThreadNode, doc: &Document) -> Anchor {
        if let Some(anchor) = self.anchors.get(&node.id) {
            return anchor.clone();
        }
        let quote = node.place.quote.clone();
        let found = match &node.place.parent {
            Some(parent) => self.message(&parent.id).and_then(|range| {
                passage::find_near(&self.chars_in(&range), &doc.source, &quote, parent.quote_offset)
            }),
            None => passage::find(&self.chars, &doc.source, &quote, 0),
        };
        match found {
            Some(range) => {
                let block = doc.block_containing(range.start);
                let range = SourceRange {
                    start: range.start,
                    end: range.end,
                    version: blob_sha(doc.source.as_bytes()),
                };
                Anchor::new(quote, &doc.source, range, Kind::Comment, block)
            }
            None => {
                Anchor { original_text: quote.clone(), quote, plannotator_tui: None, other: BTreeMap::new() }
            }
        }
    }
}

/// A comment of the Inbox's view as a note: its replies flat under it, by who wrote them.
pub(super) fn note(node: &ThreadNode, anchor: Anchor) -> Annotation {
    let replies = node.replies.iter().map(|reply| {
        let author = tree::author(reply);
        Reply {
            id: reply.id.clone(),
            annotation_id: node.id.clone(),
            body: reply.text.clone(),
            author: Some(author),
            author_name: reply.author_name.clone(),
            created_at: reply.at.clone(),
            updated_at: reply.at.clone(),
            other: BTreeMap::new(),
        }
    });
    Annotation {
        id: node.id.clone(),
        document_id: String::new(),
        anchor,
        body: node.text.clone(),
        author: None,
        author_name: None,
        state: if node.state == ThreadState::Resolved { State::Resolved } else { State::Open },
        attachments: Vec::new(),
        plannotator_tui: NoteExtras::default(),
        created_at: String::new(),
        updated_at: String::new(),
        replies: replies.collect(),
        other: BTreeMap::new(),
    }
}
