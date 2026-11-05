//! The review loop over a file or a folder sent to an agent in Herdr: one review, named by
//! the file's or the folder's real path, each file's notes the journal's comments on it
//! (`Target::File`), drafts until `E` sends the round. The tree and the Review menu count the
//! review's comments by file. The menu's actions on the notes a file's own record keeps
//! (resend all, finish, undo, the archive) stay with reviews outside the loop. An approval
//! carries the drafts as its notes, as it carries unsent notes today, and closes the review.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plannotator_tui_journal::{Anchor as On, ReviewTarget, SendTo, ThreadView, Viewed};
use plannotator_tui_schema::Provenance;

use super::super::App;
use super::super::feedback::{Feedback, ReviewCounts, SendScope};
use super::{ReviewLoop, keep_in_journal};
use crate::doc::Document;
use crate::gate::Gate;
use crate::journal::{Review, Shared, Target, real_path};
use crate::keys::Action;
use crate::layout::DocLayout;
use crate::store::Store;
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
        self.paths_changed();
    }

    /// The review of a file or a folder, when the review loop runs over one.
    pub(in crate::app) fn path_review(&self) -> Option<&Review> {
        let review = &self.review_loop.as_ref()?.review;
        matches!(review.target, ReviewTarget::File { .. }).then_some(review)
    }

    /// The view of a file's or a folder's review, as last read.
    fn path_view(&self) -> Option<&ThreadView> {
        self.path_review()?;
        let open = self.open.store.journal_view().map(|(_, view)| view);
        open.or_else(|| self.review_loop.as_ref()?.view.as_ref())
    }

    /// The comments of a file's or a folder's review on each of its files, by real path:
    /// drafts waiting, and sent. `None` without the review loop over a file or a folder.
    fn path_counts(&self) -> Option<HashMap<PathBuf, ReviewCounts>> {
        self.path_review()?;
        let mut counts: HashMap<PathBuf, ReviewCounts> = HashMap::new();
        for node in self.path_view().into_iter().flat_map(|view| &view.threads) {
            let (On::File { path, .. }, None) = (&node.place.on, &node.place.parent) else { continue };
            let count = counts.entry(real_path(Path::new(path))).or_default();
            if node.draft {
                count.pending += 1;
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
        let (root, real) = (tree.root().to_path_buf(), real_path(tree.root()));
        tree.set_counts(|path| {
            let at = path.strip_prefix(&root).map_or_else(|_| path.to_path_buf(), |rest| real.join(rest));
            counts.get(&at).map_or(0, |count| count.pending + count.sent)
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

    /// The Review menu's actions over the notes a file's record keeps do not apply to the
    /// journal's comments: the footer says so, and what does.
    pub(in crate::app) fn path_menu_off(&mut self) -> bool {
        if self.path_review().is_none() {
            return false;
        }
        let resolve = self.keys.hint(Action::Resolve);
        self.status = Some(format!(
            "review loop: {resolve} resolves a thread; resend all, finish, undo and the archive are for reviews outside the loop"
        ));
        true
    }

    /// What an approval carries over a file's or a folder's review: its drafts, by file, as
    /// today's feedback names unsent notes. `None` without the review loop over one.
    pub(in crate::app) fn path_approval(&self) -> Option<Result<Feedback>> {
        let review = self.path_review()?;
        let view = self.path_view()?;
        Some(self.drafts_feedback(review, view))
    }

    fn drafts_feedback(&self, review: &Review, view: &ThreadView) -> Result<Feedback> {
        let mut files: Vec<PathBuf> = view
            .threads
            .iter()
            .filter(|node| node.draft && node.place.parent.is_none())
            .filter_map(|node| match &node.place.on {
                On::File { path, .. } => Some(real_path(Path::new(path))),
                _ => None,
            })
            .collect();
        files.sort();
        files.dedup();
        let key = self.open.store.journal_key().unwrap_or_default().to_owned();
        let mut feedback = Feedback { ids: self.ids_in_feedback, ..Feedback::default() };
        let open = match &self.open.source.provenance {
            Provenance::File { path } => Some(real_path(path)),
            _ => None,
        };
        let root = self.tree.as_ref().map(|tree| real_path(tree.root()));
        for path in files {
            let name = match &root {
                Some(root) => path.strip_prefix(root).unwrap_or(&path).display().to_string(),
                None => self.open.source.name.clone(),
            };
            if open.as_ref() == Some(&path) {
                let store = self.open.store.clone();
                feedback.add(Some(path), None, &name, &self.open.doc, store, SendScope::Pending);
                continue;
            }
            let content =
                std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let doc = Document::parse(content);
            let layout = DocLayout::build(&doc, self.open.layout.width);
            let viewed = Viewed { review: key.clone(), view: view.clone() };
            let target = Target::File { path: path.clone() };
            let store = Store::journal_viewed(review.clone(), target, viewed, &doc, &layout);
            feedback.add(Some(path), None, &name, &doc, store, SendScope::Pending);
        }
        // A folder's feedback has always ended each file's block with one more newline.
        if self.tree.is_some() && !feedback.text.is_empty() {
            feedback.text.push('\n');
        }
        Ok(feedback)
    }

    /// The approved review ends: its journal goes, as a screen's Approve removes it.
    pub(in crate::app) fn close_path_review(&mut self) -> bool {
        let Some(review) = self.path_review().cloned() else { return false };
        if let Err(err) = review.journal.close(&review.target) {
            self.status = Some(format!("review loop: the approved review stays in the journal: {err}"));
        }
        true
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
        }
        self.paths_changed();
    }
}
