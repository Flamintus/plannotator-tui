//! Comments kept by the Inbox (`Backing::Inbox`). The Inbox is the record: this store
//! holds the comments on one document as the Inbox's thread view last listed them, and
//! writes what changes one action at a time through the Inbox's routes, never a whole
//! record, since another client may have written since.
//!
//! A save compares the notes with that listing: a new note is a new comment, a changed
//! draft the same comment again, a removed draft removed. What the Inbox refuses (an edit
//! or removal of a sent comment, an image) is kept for the footer, and the next read of
//! the view, which every write asks for, puts the notes back as the Inbox has them.
//!
//! Which of the view's comments are on the document, and where, is `words.rs`.

mod words;

use std::collections::HashMap;

use anyhow::{Context, Result};
use plannotator_tui_inbox::{ThreadNode, ThreadView};
use plannotator_tui_schema::{Anchor, Annotation, Kind, Resolution};

use super::{Backing, Store};
use crate::doc::Document;
use crate::inbox::{Shared, Target};
use crate::layout::DocLayout;

/// What an Inbox-backed store knows besides its notes.
#[derive(Clone)]
pub(crate) struct Kept {
    inbox: Shared,
    target: Target,
    /// The comments on this document as the Inbox last listed them: what a save compares with.
    listed: Vec<Listed>,
    /// The whole thread's view as last read: its round, its drafts, what the flow draws.
    view: Option<ThreadView>,
    /// Where this review put the comments it made, by id: they stay on the words selected,
    /// even where the same words come earlier.
    anchors: HashMap<String, Anchor>,
    /// The document's rendered characters with their source offsets, as last laid out:
    /// where quotes are found.
    chars: Vec<(char, usize)>,
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

    /// Comment `annotation` written as `text`: listed as the Inbox now has it.
    fn wrote(&mut self, annotation: &Annotation, text: String) {
        if let Some(listed) = self.listed.iter_mut().find(|listed| listed.id == annotation.id) {
            listed.text = text;
        } else {
            self.anchors.insert(annotation.id.clone(), annotation.anchor.clone());
            self.listed.push(Listed { id: annotation.id.clone(), text, draft: true });
        }
    }
}

impl Store {
    /// The notes on `doc`, Inbox document `target`, as the Inbox's view lists them.
    pub(crate) fn inbox(inbox: Shared, target: Target, doc: &Document, layout: &DocLayout) -> Result<Self> {
        let kept = Kept {
            inbox,
            target,
            listed: Vec::new(),
            view: None,
            anchors: HashMap::new(),
            chars: Vec::new(),
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
        kept.chars = layout.rendered_chars(&doc.source, &(0..doc.source.len()));
        let nodes: Vec<&ThreadNode> = kept.comments(&view);
        kept.listed = nodes
            .iter()
            .map(|node| Listed { id: node.id.clone(), text: node.text.clone(), draft: node.draft })
            .collect();
        self.annotations = nodes.iter().map(|node| words::note(node, kept.anchor(node, doc))).collect();
        kept.view = Some(view);
        self.resolve_all(doc);
        Ok(())
    }

    pub(crate) fn is_inbox(&self) -> bool {
        matches!(self.backing, Backing::Inbox(_))
    }

    /// The Inbox, the thread and its view as last read, for an Inbox-backed store.
    pub(crate) fn inbox_thread(&self) -> Option<(&Shared, &str, &ThreadView)> {
        let Backing::Inbox(kept) = &self.backing else { return None };
        Some((&kept.inbox, kept.target.thread_id(), kept.view.as_ref()?))
    }

    /// What an Inbox-backed store's document is.
    pub(crate) fn inbox_target(&self) -> Option<&Target> {
        let Backing::Inbox(kept) = &self.backing else { return None };
        Some(&kept.target)
    }

    /// The notes whose words are no longer found in the document.
    pub(crate) fn orphaned(&self) -> Vec<&Annotation> {
        let notes = self.annotations.iter().zip(&self.resolved);
        notes.filter(|(_, resolution)| **resolution == Resolution::Orphan).map(|(note, _)| note).collect()
    }

    /// Have the Inbox's view read again at the next wake.
    pub(crate) fn mark_inbox_stale(&mut self) {
        if let Backing::Inbox(kept) = &mut self.backing {
            kept.stale = true;
        }
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
