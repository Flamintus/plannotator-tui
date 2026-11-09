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
//! The sub-threads under the document's comments, on passages of their messages drawn in the
//! threads' boxes, are notes of this store too: noted as a note on the document's words is,
//! of the same kinds, listed after their thread's note (`notes`), written to the journal the
//! same way; they have no place in the document's words.
//!
//! Which of the view's comments are on the document, and where, is `words.rs`.

mod words;

pub(crate) use words::{draft_note, kind_of};

use std::collections::HashMap;
use std::ops::Range;

use anyhow::{Context, Result};
use plannotator_tui_journal::{Anchor as On, ThreadNode, ThreadView, Viewed};
use plannotator_tui_schema::{
    Anchor, Annotation, Base, Kind, Resolution, place_base, text_hash, track_place,
};

use super::{Backing, Placed, Store, local_id};
use crate::doc::Document;
use crate::journal::{Passage, Review, Target, tree};
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
    /// The notes on a passage of a message in a thread's box, by id: the sub-threads under
    /// the document's comments, which are not on its words.
    passages: HashMap<String, Passage>,
    /// The document's last block, as last laid out: where a comment whose words are no longer
    /// found is, its box under it and its bubble last.
    last_block: Option<Range<usize>>,
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
    /// Notes the journal said go at once as it wrote them (the review's Live mode): the app
    /// sends them (`take_goes_now`).
    goes_now: Vec<String>,
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
    /// The words the journal keeps for a note: its body, or what its kind says (a looks-good's
    /// words, a deletion's none, with its type), then one `Image: <path>` line per attached
    /// image, as the feedback names them.
    fn text(annotation: &Annotation) -> String {
        let words = match (annotation.body.is_empty(), annotation.anchor.kind()) {
            (true, Kind::LooksGood) => words::LOOKS_GOOD.to_owned(),
            _ => annotation.body.clone(),
        };
        words::with_images(&words, &annotation.plannotator_tui.images)
    }

    /// The sub-threads among `nodes`, this document's comments, on the passages of their
    /// messages in the threads' boxes.
    fn place_passages(&mut self, nodes: &[&ThreadNode]) {
        for node in nodes {
            let Some(parent) = &node.place.parent else { continue };
            let passage = Passage {
                parent: parent.id.clone(),
                quote: node.place.quote.clone(),
                offset: Some(parent.quote_offset),
            };
            self.passages.insert(node.id.clone(), passage);
        }
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
            passages: HashMap::new(),
            last_block: None,
            source: String::new(),
            chars: Vec::new(),
            stale: false,
            refused: Vec::new(),
            unsaved: Vec::new(),
            goes_now: Vec::new(),
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
        kept.last_block = doc.blocks.last().map(|block| block.range.clone());
        kept.chars = layout.rendered_chars(&doc.source, &(0..doc.source.len()));
        let nodes: Vec<&ThreadNode> = kept.comments(&view);
        kept.place_passages(&nodes);
        kept.listed = nodes
            .iter()
            .map(|node| Listed { id: node.id.clone(), text: node.text.clone(), draft: node.draft })
            .collect();
        let previous = self.previous.as_deref();
        self.annotations =
            nodes.iter().map(|node| words::note(node, kept.anchor(node, doc, previous))).collect();
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
        // The text shown before this one: the comments' places counted in it follow their lines
        // from it (`followed_places`), and the notes are taken again from the view with them.
        if !old.source.is_empty() && old.source != doc.source {
            self.previous = Some(old.source.clone());
            if let Some(view) = kept.view.take() {
                let previous = self.previous.as_deref();
                let nodes = kept.comments(&view);
                self.annotations =
                    nodes.iter().map(|node| words::note(node, kept.anchor(node, doc, previous))).collect();
                kept.view = Some(view);
            }
        }
        self.keep_unsaved(doc);
    }

    /// The document's comments whose place the journal counts in the text shown before this
    /// one (`text_hash`): each with its place followed to this text, counted in it now, and
    /// whether its words are gone, for the journal (`Journal::anchor`).
    pub(crate) fn followed_places(&self, doc: &Document) -> Vec<(String, On, bool)> {
        let Backing::Journal(kept) = &self.backing else { return Vec::new() };
        let (Some(view), Some(previous)) = (&kept.view, self.previous.as_deref()) else { return Vec::new() };
        let mut out = Vec::new();
        for node in kept.comments(view) {
            let On::File { path, lines_text, text_hash: Some(hash), .. } = &node.place.on else { continue };
            let Some(place) = words::journal_place(node) else { continue };
            if node.place.parent.is_some() || place_base(hash, &doc.source, Some(previous)) != Base::Previous
            {
                continue;
            }
            let tracked =
                track_place(place.lines, &place.lines_text, &node.place.quote, &doc.source, Some(previous));
            let on = On::File {
                path: path.clone(),
                lines: Some(tracked.lines.map(|line| line as u64)),
                lines_text: lines_text.clone(),
                text_hash: Some(text_hash(&doc.source)),
            };
            out.push((node.id.clone(), on, !tracked.found));
        }
        out
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

    /// The notes whose words are no longer found in the document; a note on a box's passage
    /// was never on them.
    pub(crate) fn orphaned(&self) -> Vec<&Annotation> {
        let notes = self.annotations.iter().zip(&self.resolved);
        let lost = |(note, resolution): &(&Annotation, &Resolution)| {
            **resolution == Resolution::Orphan && !self.on_passage(&note.id)
        };
        notes.filter(lost).map(|(note, _)| note).collect()
    }

    /// Whether note `id` is on a passage of a message in a thread's box, a sub-thread, rather
    /// than on the document's words.
    pub(crate) fn on_passage(&self, id: &str) -> bool {
        matches!(&self.backing, Backing::Journal(kept) if kept.passages.contains_key(id))
    }

    /// Note `passage` of a message in a thread's box, a sub-thread: a note as one on the
    /// document's words is, of any kind, written to the journal on that passage. Only a store
    /// the journal keeps has boxes to select in; any other notes nothing.
    pub(crate) fn add_on_passage(&mut self, passage: Passage, kind: Kind, body: String) -> Result<()> {
        let Backing::Journal(kept) = &mut self.backing else { return Ok(()) };
        let anchor = words::quoted(&passage.quote, kind);
        let id = local_id();
        kept.passages.insert(id.clone(), passage);
        self.push(id, anchor, body, Resolution::Orphan)
    }

    /// Every note in the document's order: each one placed in the document's words, then,
    /// under the review loop, the ones whose words are no longer found, on the last block as
    /// their boxes are; each followed by the notes on the passages of its thread's messages at
    /// any depth (its sub-threads), in the order its box draws them, which take its range.
    pub(crate) fn notes(&self) -> Vec<Placed<'_>> {
        // The notes whose quote is not in their place are at their place, as their boxes are.
        let mut placed = self.placed();
        placed.extend(self.kept_places().into_iter().map(|(place, _)| place));
        placed.sort_by_key(|note| note.range.start);
        let Backing::Journal(kept) = &self.backing else { return placed };
        let Some(view) = &kept.view else { return placed };
        let lost = kept.last_block.as_ref().map_or_else(Vec::new, |range| {
            self.orphaned().into_iter().map(|annotation| Placed { annotation, range }).collect()
        });
        let mut notes = Vec::with_capacity(self.annotations.len());
        for note in placed.into_iter().chain(lost) {
            notes.push(note);
            let Some(thread) = view.threads.iter().find(|node| node.id == note.annotation.id) else {
                continue;
            };
            let order = tree::under(thread);
            // A sub-thread is under this thread when the message its passage is of is in it; one
            // the view does not list yet (just written, or not taken) goes after the others.
            let mut subs: Vec<(usize, &Annotation)> = self
                .annotations
                .iter()
                .filter_map(|sub| {
                    let passage = kept.passages.get(&sub.id)?;
                    tree::holding(std::slice::from_ref(thread), &passage.parent)?;
                    Some((order.iter().position(|node| node.id == sub.id).unwrap_or(usize::MAX), sub))
                })
                .collect();
            subs.sort_by_key(|(at, _)| *at);
            notes.extend(subs.into_iter().map(|(_, annotation)| Placed { annotation, range: note.range }));
        }
        notes
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

    /// The notes the journal said go at once since the last call (the review's Live mode).
    pub(crate) fn take_goes_now(&mut self) -> Vec<String> {
        match &mut self.backing {
            Backing::Journal(kept) => std::mem::take(&mut kept.goes_now),
            _ => Vec::new(),
        }
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
                Some(Ok(now)) => {
                    kept.unsaved.retain(|unsaved| unsaved.id != annotation.id);
                    if now && !kept.goes_now.contains(&annotation.id) {
                        kept.goes_now.push(annotation.id.clone());
                    }
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
