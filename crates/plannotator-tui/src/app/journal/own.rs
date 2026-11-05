//! A file's own notes, kept outside the loop in its record: written to the review as drafts
//! the first time the file opens in the loop, each marked in the record so it goes once; the
//! record keeps them for when the loop is off.

use plannotator_tui_schema::Provenance;

use super::super::App;
use crate::journal::tree;
use crate::store::{Location, Store};

impl App {
    /// The open file's notes kept outside the loop, in its own record: written to the review
    /// as drafts the first time the file opens in the loop, each marked in the record so it
    /// goes once; the record keeps them for when the loop is off.
    pub(super) fn adopt_own_notes(&mut self) {
        let Provenance::File { path } = &self.open.source.provenance else { return };
        let Some((key, view)) = self
            .open
            .store
            .journal_view()
            .map(|(_, view)| view)
            .and_then(|view| Some((self.open.store.journal_key()?.to_owned(), view)))
        else {
            return;
        };
        let location = Location::for_file(&self.data_dir, &self.project, path);
        let mut own = match Store::load(&location, &self.open.doc) {
            Ok(own) => own,
            Err(err) => return self.add_status(&format!("review loop: this file's own notes: {err:#}")),
        };
        let notes: Vec<_> = own
            .outside_journal(&key)
            .into_iter()
            .filter(|note| tree::find(&view.threads, &note.id).is_none())
            .collect();
        if notes.is_empty() {
            return;
        }
        let taken = self.open.store.adopt(notes, &self.open.doc);
        if taken.is_empty() {
            return;
        }
        let note = match own.mark_in_journal(&taken, &key) {
            Ok(()) => format!(
                "review loop: {} note(s) kept outside the loop are drafts of this review",
                taken.len()
            ),
            Err(err) => format!(
                "review loop: {} note(s) are drafts of this review; their record: {err:#}",
                taken.len()
            ),
        };
        self.add_status(&note);
    }
}
