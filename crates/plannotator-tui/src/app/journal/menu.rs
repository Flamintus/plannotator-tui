//! The Review menu over a file's or a folder's review in the loop, each action adapted to the
//! journal: a finished note is a resolved thread. `F` resolves the review's sent threads that
//! are still open (drafts stay, as pending notes stay); `U` opens again what the last `F`
//! resolved; `H` lists the resolved threads, and restoring one opens it again; `R` ends a round
//! with nothing new, which names every thread still open to the agent, as resend all puts
//! every active note before it again.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plannotator_tui_journal::{Anchor as On, CommentKind, State as ThreadState, ThreadNode, ThreadView};
use plannotator_tui_schema::Provenance;

use super::super::App;
use super::super::review::ArchivedItem;
use crate::journal::real_path;
use crate::store::draft_note;

/// The top-level threads of `view` on file `path` (a real path).
fn on_file<'v>(view: &'v ThreadView, path: &Path) -> impl Iterator<Item = &'v ThreadNode> {
    view.threads.iter().filter(move |node| {
        node.place.parent.is_none()
            && matches!(&node.place.on, On::File { path: on, .. } if real_path(Path::new(on)) == path)
    })
}

/// The sent threads still open of `node` and under it, at any depth: what a finish resolves.
fn open_sent(node: &ThreadNode, out: &mut Vec<String>) {
    if !node.draft && node.state == ThreadState::Open {
        out.push(node.id.clone());
    }
    let subs = node.sub_threads.iter().chain(node.replies.iter().flat_map(|reply| &reply.sub_threads));
    for sub in subs {
        open_sent(sub, out);
    }
}

impl App {
    /// The files of a file's or a folder's review in the loop: the open one and each one the
    /// review has comments on, by real path. `None` without the loop over one.
    pub(in crate::app) fn path_files(&self) -> Option<Vec<PathBuf>> {
        let view = self.path_view()?;
        let mut files: Vec<PathBuf> = view
            .threads
            .iter()
            .filter_map(|node| match (&node.place.on, &node.place.parent) {
                (On::File { path, .. }, None) => Some(real_path(Path::new(path))),
                _ => None,
            })
            .collect();
        if let Provenance::File { path } = &self.open.source.provenance {
            files.push(real_path(path));
        }
        let root = self.tree.as_ref().map(|tree| real_path(tree.root()));
        files.retain(|file| root.as_ref().is_none_or(|root| file.starts_with(root)));
        files.sort();
        files.dedup();
        Some(files)
    }

    /// `F` on file `path`: its sent threads still open, resolved; their ids, for `U`. `None`
    /// without the loop over a file or a folder.
    pub(in crate::app) fn path_finish(&mut self, path: &Path) -> Option<Result<Vec<String>>> {
        let review = self.path_review()?.clone();
        let view = self.path_view()?;
        let mut ids = Vec::new();
        for node in on_file(view, path) {
            open_sent(node, &mut ids);
        }
        let mut resolved = Vec::new();
        for id in ids {
            if let Err(err) = review.journal.resolve(&review.target, &id, true) {
                self.refresh_journal();
                return Some(Err(anyhow::Error::new(err)).with_context(|| format!("resolving {id}")));
            }
            resolved.push(id);
        }
        self.refresh_journal();
        Some(Ok(resolved))
    }

    /// `U` and a restore from `H`: threads `ids` opened again. `None` without the loop over a
    /// file or a folder.
    pub(in crate::app) fn path_reopen(&mut self, ids: &[String]) -> Option<Result<usize>> {
        let review = self.path_review()?.clone();
        let mut opened = 0;
        for id in ids {
            if let Err(err) = review.journal.resolve(&review.target, id, false) {
                self.refresh_journal();
                return Some(Err(anyhow::Error::new(err)).with_context(|| format!("opening {id} again")));
            }
            opened += 1;
        }
        self.refresh_journal();
        Some(Ok(opened))
    }

    /// `H`: the review's resolved threads, by file, newest last, as the archive lists
    /// finished notes. `None` without the loop over a file or a folder.
    pub(in crate::app) fn path_archive(&self) -> Option<Vec<ArchivedItem>> {
        let view = self.path_view()?;
        let files = self.path_files()?;
        let mut items = Vec::new();
        for path in files {
            for node in on_file(view, &path).filter(|node| node.state == ThreadState::Resolved) {
                let deletion = node.kind == CommentKind::Deletion;
                let annotation = draft_note(&node.id, &node.place.quote, &node.text, deletion);
                items.push(ArchivedItem { path: path.clone(), annotation });
            }
        }
        Some(items)
    }

    /// `R`: a round with nothing new, which names the threads still open to the agent. `false`
    /// without the loop over a file or a folder.
    pub(in crate::app) fn path_resend_all(&mut self) -> bool {
        if self.path_review().is_none() {
            return false;
        }
        self.send_journal_round(true);
        true
    }
}
