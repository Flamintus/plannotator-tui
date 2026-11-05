//! Comments kept by the Inbox (`Backing::Inbox`). The Inbox is the record: this store
//! holds the comments on one document as the Inbox's thread view last listed them, and
//! writes what changes one action at a time through the Inbox's routes, never a whole
//! record, since another client may have written since.
//!
//! A save compares the notes with that listing: a new note is a new comment, a changed
//! draft the same comment again, a removed draft removed. What the Inbox refuses (an edit
//! or removal of a sent comment, an image) is kept for the footer, and the next read of
//! the view, which every write asks for, puts the notes back as the Inbox has them.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use plannotator_tui_inbox::{
    Anchor as On, Author, FileVersion, NewComment, State as ThreadState, ThreadNode, ThreadView,
};
use plannotator_tui_schema::{Anchor, Annotation, Kind, NoteExtras, Reply, SourceRange, State, blob_sha};

use super::{Backing, Store};
use crate::doc::Document;
use crate::inbox::{Shared, Target, passage};
use crate::layout::DocLayout;

/// What an Inbox-backed store knows besides its notes.
#[derive(Clone)]
pub(crate) struct Kept {
    inbox: Shared,
    target: Target,
    /// The comments on this document as the Inbox last listed them: what a save compares with.
    listed: Vec<Listed>,
    /// Where this review put the comments it made, by id: they stay on the words selected,
    /// even where the same words come earlier.
    anchors: HashMap<String, Anchor>,
    /// Written since the view was read: read it again.
    stale: bool,
    /// What the Inbox refused since the footer last said.
    refused: Vec<String>,
}

#[derive(Debug, Clone)]
struct Listed {
    id: String,
    text: String,
    draft: bool,
}

impl std::fmt::Debug for Kept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kept")
            .field("target", &self.target)
            .field("listed", &self.listed)
            .finish_non_exhaustive()
    }
}

impl Kept {
    /// The words of a note the Inbox keeps: its body, or what its kind says.
    fn text(annotation: &Annotation) -> String {
        match (annotation.body.is_empty(), annotation.anchor.kind()) {
            (false, _) | (true, Kind::Comment) => annotation.body.clone(),
            (true, Kind::LooksGood) => "Looks good.".to_owned(),
            (true, Kind::Delete) => "Delete this.".to_owned(),
        }
    }

    /// Write `annotation` as a new comment, or as an edit of its draft.
    fn comment(&self, annotation: &Annotation) -> Result<()> {
        let comment = NewComment::new(&annotation.id, annotation.anchor.rendered(), Self::text(annotation));
        match &self.target {
            Target::Message { message_id, .. } => self.inbox.comment_on_message(message_id, &comment),
            Target::Attachment { attachment_id, version, .. } => {
                self.inbox.comment_on_attachment(attachment_id, version, &comment)
            }
        }
        .map(drop)
        .with_context(|| format!("comment {}", annotation.id))
    }

    /// Comment `annotation` written as `text`: listed as the Inbox now has it.
    fn wrote(&mut self, annotation: &Annotation, text: String) {
        if let Some(listed) = self.listed.iter_mut().find(|listed| listed.id == annotation.id) {
            listed.text = text;
        } else {
            self.anchors.insert(annotation.id.clone(), annotation.anchor.clone());
            self.listed.push(Listed { id: annotation.id.clone(), text, draft: true });
        }
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
    /// place the words as drawn hold it, else an anchor that resolves to nothing.
    fn anchor(&self, node: &ThreadNode, doc: &Document, chars: &[(char, usize)]) -> Anchor {
        if let Some(anchor) = self.anchors.get(&node.id) {
            return anchor.clone();
        }
        let quote = node.place.quote.clone();
        match passage::find(chars, &doc.source, &quote, 0) {
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
fn note(node: &ThreadNode, anchor: Anchor) -> Annotation {
    let replies = node.replies.iter().map(|reply| {
        let author = match &reply.author {
            Author::Person => "You".to_owned(),
            Author::Agent { host, .. } => host.clone().unwrap_or_else(|| "agent".to_owned()),
        };
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

impl Store {
    /// The notes on `doc`, Inbox document `target`, as the Inbox's view lists them.
    pub(crate) fn inbox(inbox: Shared, target: Target, doc: &Document, layout: &DocLayout) -> Result<Self> {
        let kept = Kept {
            inbox,
            target,
            listed: Vec::new(),
            anchors: HashMap::new(),
            stale: false,
            refused: Vec::new(),
        };
        let mut store = Self::transient();
        store.backing = Backing::Inbox(Box::new(kept));
        store.refresh_inbox(doc, layout)?;
        Ok(store)
    }

    /// Read the thread's view again and take this document's comments from it.
    pub(crate) fn refresh_inbox(&mut self, doc: &Document, layout: &DocLayout) -> Result<()> {
        let Backing::Inbox(kept) = &mut self.backing else { return Ok(()) };
        kept.stale = false;
        let view: ThreadView =
            kept.inbox.thread_view(kept.target.thread_id()).context("reading the Inbox's comments")?;
        let chars = layout.rendered_chars(&doc.source, &(0..doc.source.len()));
        let nodes: Vec<&ThreadNode> = view.threads.iter().filter(|node| kept.is_on_this(node)).collect();
        kept.listed = nodes
            .iter()
            .map(|node| Listed { id: node.id.clone(), text: node.text.clone(), draft: node.draft })
            .collect();
        self.annotations = nodes.iter().map(|node| note(node, kept.anchor(node, doc, &chars))).collect();
        self.resolve_all(doc);
        Ok(())
    }

    pub(crate) fn is_inbox(&self) -> bool {
        matches!(self.backing, Backing::Inbox(_))
    }

    /// Whether a write asks for the Inbox's view to be read again.
    pub(crate) fn inbox_stale(&self) -> bool {
        matches!(&self.backing, Backing::Inbox(kept) if kept.stale)
    }

    /// What the Inbox refused since the last call.
    pub(crate) fn take_refusals(&mut self) -> Vec<String> {
        match &mut self.backing {
            Backing::Inbox(kept) => std::mem::take(&mut kept.refused),
            _ => Vec::new(),
        }
    }

    /// `save` for the stores that write no file: nothing for a transient one; for the
    /// Inbox's, each note that changed since the Inbox last listed them, one write each.
    pub(super) fn save_to_inbox(&mut self) {
        let Backing::Inbox(kept) = &mut self.backing else { return };
        for annotation in &self.annotations {
            let text = Kept::text(annotation);
            let listed = kept.listed.iter().find(|listed| listed.id == annotation.id);
            let written = if annotation.plannotator_tui.images.is_empty() {
                match listed.map(|listed| (listed.text == text, listed.draft)) {
                    Some((true, _)) => continue,
                    None | Some((false, true)) => kept.comment(annotation),
                    Some((false, false)) => {
                        Err(anyhow::anyhow!("{} is sent: reply under it rather than edit it", annotation.id))
                    }
                }
            } else {
                Err(anyhow::anyhow!("the Inbox takes no images: {} keeps its words only", annotation.id))
            };
            kept.stale = true;
            match written {
                Ok(()) => kept.wrote(annotation, text),
                Err(err) => kept.refused.push(format!("{err:#}")),
            }
        }
        let gone: Vec<Listed> = kept
            .listed
            .iter()
            .filter(|listed| !self.annotations.iter().any(|a| a.id == listed.id))
            .cloned()
            .collect();
        for listed in gone {
            kept.stale = true;
            let removed = if listed.draft {
                kept.inbox.remove(&listed.id).map(drop).with_context(|| format!("removing {}", listed.id))
            } else {
                Err(anyhow::anyhow!("{} is sent and stays: resolve it instead", listed.id))
            };
            match removed {
                Ok(()) => kept.listed.retain(|kept| kept.id != listed.id),
                Err(err) => kept.refused.push(format!("{err:#}")),
            }
        }
    }
}
