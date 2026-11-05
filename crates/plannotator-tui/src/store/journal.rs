//! Comments kept by the review journal (`Backing::Journal`). The journal is the record: this
//! store holds the comments on one document as the review's view last listed them, and
//! writes what changes one action at a time through the CLI, never a whole record, since the
//! agent or another client may have written since.
//!
//! A save compares the notes with that listing: a new note is a new comment, a changed draft
//! the same comment again, a removed draft removed. What the journal refuses (an edit or
//! removal of a sent comment) is kept for the footer, and the next read of the view, which
//! every write asks for, puts the notes back as the journal has them.
//!
//! Which of the view's comments are on the document, and where, is `words.rs`.

mod words;

pub(crate) use words::draft_note;

use std::collections::HashMap;

use anyhow::{Context, Result};
use plannotator_tui_journal::{ThreadNode, ThreadView, Viewed};
use plannotator_tui_schema::{Anchor, Annotation, Kind, Resolution};

use super::{Backing, Store};
use crate::doc::Document;
use crate::journal::{Review, Target};
use crate::layout::DocLayout;

/// What a journal-backed store knows besides its notes.
#[derive(Clone)]
pub(crate) struct Kept {
    review: Review,
    target: Target,
    /// The review's key, as the view last named it: where its journal is.
    key: Option<String>,
    /// The comments on this document as the journal last listed them: what a save compares with.
    listed: Vec<Listed>,
    /// The whole review's view as last read: its round, its drafts, what the flow draws.
    view: Option<ThreadView>,
    /// Where this review put the comments it made, by id: they stay on the words selected,
    /// even where the same words come earlier, with the kind of note they are.
    anchors: HashMap<String, Anchor>,
    /// The document's words and its rendered characters with their source offsets, as last
    /// laid out: where quotes are found.
    source: String,
    chars: Vec<(char, usize)>,
    /// Written since the view was read: read it again.
    stale: bool,
    /// What the journal refused since the footer last said.
    refused: Vec<String>,
    /// Notes whose last write the journal did not take, as the person left them: kept here,
    /// words and all, until a write takes them or the person removes them.
    unsaved: Vec<Annotation>,
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
    /// The words the journal keeps for a note: its body, or what its kind says, then one
    /// `Image: <path>` line per attached image, as the feedback names them.
    fn text(annotation: &Annotation) -> String {
        let words = match (annotation.body.is_empty(), annotation.anchor.kind()) {
            (true, Kind::LooksGood) => "Looks good.".to_owned(),
            _ => annotation.body.clone(),
        };
        words::with_images(&words, &annotation.plannotator_tui.images)
    }

    /// Comment `annotation` written as `text`: listed as the journal now has it.
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
    /// The notes on `doc`, document `target` of `review`, as the review's view lists them.
    pub(crate) fn journal(
        review: Review,
        target: Target,
        doc: &Document,
        layout: &DocLayout,
    ) -> Result<Self> {
        let kept = Kept {
            review,
            target,
            key: None,
            listed: Vec::new(),
            view: None,
            anchors: HashMap::new(),
            source: String::new(),
            chars: Vec::new(),
            stale: false,
            refused: Vec::new(),
            unsaved: Vec::new(),
        };
        let mut store = Self::transient();
        store.backing = Backing::Journal(Box::new(kept));
        store.refresh_journal(doc, layout)?;
        Ok(store)
    }

    /// Read the review's view again and take this document's comments from it.
    pub(crate) fn refresh_journal(&mut self, doc: &Document, layout: &DocLayout) -> Result<()> {
        let Backing::Journal(kept) = &mut self.backing else { return Ok(()) };
        // Still stale when the read fails: the next wake tries again.
        let viewed =
            kept.review.journal.view(&kept.review.target).context("reading the review's comments")?;
        self.take_view(viewed, doc, layout);
        Ok(())
    }

    /// This document's comments from the review's view `viewed`.
    fn take_view(&mut self, viewed: Viewed, doc: &Document, layout: &DocLayout) {
        let Backing::Journal(kept) = &mut self.backing else { return };
        kept.stale = false;
        kept.key = Some(viewed.review);
        let view = viewed.view;
        kept.source.clone_from(&doc.source);
        kept.chars = layout.rendered_chars(&doc.source, &(0..doc.source.len()));
        let nodes: Vec<&ThreadNode> = kept.comments(&view);
        kept.listed = nodes
            .iter()
            .map(|node| Listed { id: node.id.clone(), text: node.text.clone(), draft: node.draft })
            .collect();
        self.annotations = nodes.iter().map(|node| words::note(node, kept.anchor(node, doc))).collect();
        kept.view = Some(view);
        self.keep_unsaved(doc);
    }

    /// The notes the journal did not take, as the person left them, over what it lists; one
    /// it now lists as left is saved.
    fn keep_unsaved(&mut self, doc: &Document) {
        let Backing::Journal(kept) = &mut self.backing else { return };
        let listed = &kept.listed;
        kept.unsaved.retain(|unsaved| {
            !listed.iter().any(|listed| listed.id == unsaved.id && listed.text == Kept::text(unsaved))
        });
        for unsaved in &kept.unsaved {
            match self.annotations.iter_mut().find(|note| note.id == unsaved.id) {
                Some(note) => unsaved.clone_into(note),
                None => self.annotations.push(unsaved.clone()),
            }
        }
        self.resolve_all(doc);
    }

    /// Take over what `from`, the store of the same document written again, knew beyond the
    /// journal: where this review put its comments, and the notes the journal did not take.
    pub(crate) fn carry_over(&mut self, from: &Store, doc: &Document) {
        let (Backing::Journal(kept), Backing::Journal(old)) = (&mut self.backing, &from.backing) else {
            return;
        };
        for (id, anchor) in &old.anchors {
            kept.anchors.entry(id.clone()).or_insert_with(|| anchor.clone());
        }
        kept.unsaved.clone_from(&old.unsaved);
        self.keep_unsaved(doc);
    }

    pub(crate) fn is_journal(&self) -> bool {
        matches!(self.backing, Backing::Journal(_))
    }

    /// The review and its view as last read, for a journal-backed store.
    pub(crate) fn journal_view(&self) -> Option<(&Review, &ThreadView)> {
        let Backing::Journal(kept) = &self.backing else { return None };
        Some((&kept.review, kept.view.as_ref()?))
    }

    /// The review's key, once its view was read.
    pub(crate) fn journal_key(&self) -> Option<&str> {
        let Backing::Journal(kept) = &self.backing else { return None };
        kept.key.as_deref()
    }

    /// For a store the journal keeps: whether note `id` waits for a send (a draft, or a note
    /// the journal has not taken), rather than being a comment a send carried. `None` for
    /// any other store.
    pub(crate) fn journal_waits(&self, id: &str) -> Option<bool> {
        let Backing::Journal(kept) = &self.backing else { return None };
        let listed = kept.listed.iter().find(|listed| listed.id == id);
        Some(listed.is_none_or(|listed| listed.draft) || kept.unsaved.iter().any(|note| note.id == id))
    }

    /// Whether note `id` is kept here only, the journal having not taken its last write.
    pub(crate) fn journal_unsaved(&self, id: &str) -> bool {
        matches!(&self.backing, Backing::Journal(kept) if kept.unsaved.iter().any(|note| note.id == id))
    }

    /// What a journal-backed store's document is.
    pub(crate) fn journal_target(&self) -> Option<&Target> {
        let Backing::Journal(kept) = &self.backing else { return None };
        Some(&kept.target)
    }

    /// The notes whose words are no longer found in the document.
    pub(crate) fn orphaned(&self) -> Vec<&Annotation> {
        let notes = self.annotations.iter().zip(&self.resolved);
        notes.filter(|(_, resolution)| **resolution == Resolution::Orphan).map(|(note, _)| note).collect()
    }

    /// Have the review's view read again at the next wake.
    pub(crate) fn mark_journal_stale(&mut self) {
        if let Backing::Journal(kept) = &mut self.backing {
            kept.stale = true;
        }
    }

    /// Whether a write asks for the review's view to be read again.
    pub(crate) fn journal_stale(&self) -> bool {
        matches!(&self.backing, Backing::Journal(kept) if kept.stale)
    }

    /// What the journal refused since the last call.
    pub(crate) fn take_refusals(&mut self) -> Vec<String> {
        match &mut self.backing {
            Backing::Journal(kept) => std::mem::take(&mut kept.refused),
            _ => Vec::new(),
        }
    }

    /// `save` for the stores that write no file: nothing for a transient one; for the
    /// journal's, each note that changed since the journal last listed them, one write each.
    pub(super) fn save_to_journal(&mut self) {
        let Backing::Journal(kept) = &mut self.backing else { return };
        for annotation in &self.annotations {
            let text = Kept::text(annotation);
            let listed = kept.listed.iter().find(|listed| listed.id == annotation.id);
            let written = match listed.map(|listed| (listed.text == text, listed.draft)) {
                Some((true, _)) => None,
                None | Some((false, true)) => Some(kept.comment(annotation, &text)),
                Some((false, false)) => {
                    kept.refused
                        .push(format!("{} is sent: reply under it rather than edit it", annotation.id));
                    // Read the view again: the journal's words come back.
                    kept.stale = true;
                    None
                }
            };
            kept.stale |= written.is_some();
            match written {
                Some(Ok(())) => {
                    kept.unsaved.retain(|unsaved| unsaved.id != annotation.id);
                    kept.wrote(annotation, text);
                }
                // Kept here, its words with it, until a write takes it or the person removes it.
                Some(Err(err)) => {
                    kept.refused.push(format!("{err:#}"));
                    kept.unsaved.retain(|unsaved| unsaved.id != annotation.id);
                    kept.unsaved.push(annotation.clone());
                }
                None => {}
            }
        }
        kept.unsaved.retain(|unsaved| self.annotations.iter().any(|a| a.id == unsaved.id));
        let gone: Vec<Listed> = kept
            .listed
            .iter()
            .filter(|listed| !self.annotations.iter().any(|a| a.id == listed.id))
            .cloned()
            .collect();
        for listed in gone {
            kept.stale = true;
            let removed = if listed.draft {
                let review = &kept.review;
                review
                    .journal
                    .remove(&review.target, &listed.id)
                    .with_context(|| format!("removing {}", listed.id))
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
