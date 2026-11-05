//! The review loop over a file or a folder sent to an agent in Herdr: one review, named by
//! the file's or the folder's real path, each file's notes the journal's comments on it
//! (`Target::File`), drafts until `E` sends the round. The tree and the Review menu count the
//! review's comments by file, and the Review menu acts on them (`menu`). An approval carries
//! every draft as its notes, as it carries unsent notes today, and closes the review.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use plannotator_tui_journal::{
    Anchor as On, Author, CommentKind, ReviewTarget, SendTo, State as ThreadState, ThreadNode, ThreadView,
};
use plannotator_tui_schema::{Annotation, Provenance};

use super::super::App;
use super::super::feedback::{Feedback, ReviewCounts};
use super::{ReviewLoop, keep_in_journal};
use crate::export;
use crate::gate::Gate;
use crate::journal::{Review, Shared, real_path};
use crate::keys::Action;
use crate::store::draft_note;
use crate::tree::Tree;

impl App {
    /// Review this file or folder through `journal` when the review loop is on, its rounds
    /// sent to `to`; say why not when it cannot run; do nothing when it is off.
    pub(crate) fn review_path_in_journal(&mut self, journal: Result<Option<(Shared, SendTo)>>) {
        let (journal, to) = match journal {
            Ok(None) => return,
            Ok(Some(journal)) => journal,
            Err(err) => return self.loop_off(&err),
        };
        if self.gate == Gate::Decision {
            return self.loop_off(&anyhow::anyhow!("a review that ends with its first send has no rounds"));
        }
        let root = match (&self.tree, &self.open.source.provenance) {
            (Some(tree), _) => tree.root().to_path_buf(),
            (None, Provenance::File { path }) if !self.open.source.transient => path.clone(),
            _ => return,
        };
        let mut review_loop = ReviewLoop::new(Review::of_path(journal, &real_path(&root), to));
        keep_in_journal(Some(&mut review_loop), &mut self.open);
        if let Some(why) = review_loop.unlisted.take() {
            return self.loop_off(&anyhow::anyhow!(why));
        }
        if !self.open.store.is_journal() {
            // A folder opened on no file of its own: the counts come from the view as it is.
            let review = &review_loop.review;
            match review.journal.view(&review.target) {
                Ok(viewed) => review_loop.view = Some(viewed.view),
                Err(err) => return self.loop_off(&anyhow::Error::new(err)),
            }
        }
        self.review_loop = Some(review_loop);
        self.strand_pending();
        self.adopt_own_notes();
        self.paths_changed();
    }

    /// A folder opened on no file of its own: its review's view read again, and what it
    /// counts with it. False for any other document, whose store reads its view.
    pub(in crate::app) fn refresh_path_view(&mut self) -> bool {
        if self.open.store.is_journal() {
            return false;
        }
        let Some(review) = self.path_review().cloned() else { return false };
        match review.journal.view(&review.target) {
            Ok(viewed) => {
                if let Some(review_loop) = self.review_loop.as_mut() {
                    review_loop.view = Some(viewed.view);
                }
            }
            Err(err) => self.status = Some(format!("review loop: {err}")),
        }
        self.paths_changed();
        true
    }

    /// The review of a file or a folder, when the review loop runs over one.
    pub(in crate::app) fn path_review(&self) -> Option<&Review> {
        let review = &self.review_loop.as_ref()?.review;
        matches!(review.target, ReviewTarget::File { .. }).then_some(review)
    }

    /// The view of a file's or a folder's review, as last read.
    pub(super) fn path_view(&self) -> Option<&ThreadView> {
        self.path_review()?;
        let open = self.open.store.journal_view().map(|(_, view)| view);
        open.or_else(|| self.review_loop.as_ref()?.view.as_ref())
    }

    /// The comments of a file's or a folder's review on each of its files, by real path:
    /// drafts waiting, sent and still open, and resolved (the menu's archive). `None` without
    /// the review loop over a file or a folder.
    pub(in crate::app) fn path_counts(&self) -> Option<HashMap<PathBuf, ReviewCounts>> {
        self.path_review()?;
        let mut counts: HashMap<PathBuf, ReviewCounts> = HashMap::new();
        for node in self.path_view().into_iter().flat_map(|view| &view.threads) {
            let (On::File { path, .. }, None) = (&node.place.on, &node.place.parent) else { continue };
            let count = counts.entry(real_path(Path::new(path))).or_default();
            if node.draft {
                count.pending += 1;
            } else if node.state == ThreadState::Resolved {
                count.archived += 1;
            } else {
                count.sent += 1;
            }
        }
        Some(counts)
    }

    /// The tree's counts from the review's comments; false without the review loop over a
    /// folder, whose counts come from each file's record.
    pub(in crate::app) fn path_tree_counts(&self, tree: &mut Tree) -> bool {
        let Some(counts) = self.path_counts() else { return false };
        tree.set_counts(|path| {
            counts.get(&real_path(path)).map_or(0, |count| count.pending + count.sent + count.archived)
        });
        true
    }

    /// The folder's counts by file from the review's comments; false without the review loop
    /// over a file or a folder.
    pub(in crate::app) fn path_folder_counts(&mut self) -> bool {
        let Some(counts) = self.path_counts() else { return false };
        if self.tree.is_some() {
            self.folder_counts = counts;
        }
        true
    }

    /// After the review's view was read again: the tree and the folder's counts follow it.
    pub(in crate::app) fn paths_changed(&mut self) {
        if self.path_review().is_none() {
            return;
        }
        self.refresh_review_counts();
        self.sync_tree_counts();
        self.derive_send_state();
    }

    /// What an approval carries over a file's or a folder's review: every draft, by file, as
    /// today's feedback names unsent notes; a reply or a sub-thread on the words its thread
    /// or its passage is on. The open file's comments read as its notes do; the others as
    /// the view has them, so a file the agent removed still has its drafts said. `None`
    /// without the review loop over a file or a folder.
    pub(in crate::app) fn path_approval(&self) -> Option<Feedback> {
        let view = self.path_view()?;
        let open = match &self.open.source.provenance {
            Provenance::File { path } => Some(real_path(path)),
            _ => None,
        };
        let mut files: Vec<(PathBuf, Vec<Draft>)> = Vec::new();
        for node in &view.threads {
            let (On::File { path, lines }, None) = (&node.place.on, &node.place.parent) else { continue };
            let file = real_path(Path::new(path));
            let lines = lines.map(|[first, last]| (first as usize, last as usize));
            let mut drafts = Vec::new();
            drafts_under(node, lines, &mut drafts);
            if drafts.is_empty() {
                continue;
            }
            match files.iter_mut().find(|(known, _)| *known == file) {
                Some((_, known)) => known.extend(drafts),
                None => files.push((file, drafts)),
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let root = self.tree.as_ref().map(|tree| real_path(tree.root()));
        // The journal closes with the approval: no id there to answer under.
        let mut feedback = Feedback::default();
        for (file, drafts) in files {
            let name = match &root {
                Some(root) => file.strip_prefix(root).unwrap_or(&file).display().to_string(),
                None => self.open.source.name.clone(),
            };
            let here = open.as_ref() == Some(&file);
            let notes: Vec<_> = drafts.iter().map(|draft| (draft, draft.note())).collect();
            let entries: Vec<export::Entry<'_>> = notes
                .iter()
                .map(|(draft, note)| match self.placed_here(here, &note.id) {
                    Some(entry) => entry,
                    None => {
                        export::Entry { annotation: note, quote: draft.quote.clone(), lines: draft.lines }
                    }
                })
                .collect();
            feedback.push_entries(&name, &entries);
        }
        // A folder's feedback has always ended each file's block with one more newline.
        if self.tree.is_some() && !feedback.text.is_empty() {
            feedback.text.push('\n');
        }
        Some(feedback)
    }

    /// Note `id` of the open file, as its feedback names it, when it is placed there.
    fn placed_here(&self, here: bool, id: &str) -> Option<export::Entry<'_>> {
        if !here {
            return None;
        }
        let placed = self.open.store.placed().into_iter().find(|placed| placed.annotation.id == id)?;
        let source = &self.open.doc.source;
        Some(export::Entry {
            annotation: placed.annotation,
            quote: export::quote(source, placed.range, &[]),
            lines: Some(export::line_span(source, placed.range)),
        })
    }

    /// The approved review ends: its journal goes, as a screen's Approve removes it. `None`
    /// without the review loop over a file or a folder; `Some(false)` when the journal stayed,
    /// which the footer says.
    pub(in crate::app) fn close_path_review(&mut self) -> Option<bool> {
        let review = self.path_review().cloned()?;
        let closed = review.journal.close(&review.target);
        if let Err(err) = &closed {
            let quit = self.keys.hint(Action::Quit);
            self.status =
                Some(format!("approved \u{b7} the review stays in the journal: {err} \u{b7} {quit} closes"));
        }
        Some(closed.is_ok())
    }

    /// A file opened from the tree or reloaded, under the review loop: its notes are the
    /// journal's, or the footer says why they are not.
    pub(in crate::app) fn keep_path_in_journal(&mut self) {
        if self.path_review().is_none() {
            return;
        }
        let before = self.open.store.clone();
        keep_in_journal(self.review_loop.as_mut(), &mut self.open);
        let unlisted = self.review_loop.as_mut().and_then(|review_loop| review_loop.unlisted.take());
        if let Some(why) = unlisted {
            self.status = Some(format!("review loop: {why} \u{b7} notes on this file stay here"));
        } else {
            self.open.store.carry_over(&before, &self.open.doc);
            self.adopt_own_notes();
        }
        self.paths_changed();
    }
}

/// A draft of the review's view, as an approval names it: on the words its thread or its
/// passage is on, and the lines of a file it is on, when known.
struct Draft {
    id: String,
    quote: String,
    text: String,
    lines: Option<(usize, usize)>,
    deletion: bool,
}

impl Draft {
    fn note(&self) -> Annotation {
        draft_note(&self.id, &self.quote, &self.text, self.deletion)
    }
}

/// The drafts of `node` and under it: itself, the person's replies on its words, and the
/// sub-threads on its messages' passages (no file lines: a passage is in a message).
fn drafts_under(node: &ThreadNode, lines: Option<(usize, usize)>, out: &mut Vec<Draft>) {
    let quote = node.place.quote.clone();
    if node.draft {
        let deletion = node.kind == CommentKind::Deletion;
        out.push(Draft {
            id: node.id.clone(),
            quote: quote.clone(),
            text: node.text.clone(),
            lines,
            deletion,
        });
    }
    for reply in &node.replies {
        if reply.draft && matches!(reply.author, Author::Person) {
            let text = reply.text.clone();
            out.push(Draft { id: reply.id.clone(), quote: quote.clone(), text, lines, deletion: false });
        }
        for sub in &reply.sub_threads {
            drafts_under(sub, None, out);
        }
    }
    for sub in &node.sub_threads {
        drafts_under(sub, None, out);
    }
}
