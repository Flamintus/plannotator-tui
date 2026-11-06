//! The boxes' units of selection (`unit`): on the way from a block to the next, each paragraph
//! of a comment, a reply or a sub-thread, at any depth, as `doc::plain_blocks` parts a message's
//! words, the one a sub-thread's box goes under, is a stop of `down` and `up`, selected and drawn
//! as a block is, and so is a thread's fold line (its folded line, or the head of its box,
//! `fold`); the box's other rows (its sides, the authors, the blank rows) are stepped over as the
//! gaps between blocks are. On a paragraph the document's keys act as on a block: `comment`
//! takes a sub-thread on the paragraph whole, `move-cursor` and `visual` start on its first word,
//! and the thread keys act on its box; on a fold line the arrows, or a click, fold or unfold its
//! thread, and the thread keys act on it. On any of them in a draft's box, `edit` and `confirm`
//! edit the draft there (`draft_box`). Only while the boxes' words take a selection: otherwise
//! `down` and `up` go from block to block, as they always did. A box is brought into view as a
//! unit is, from the rail too (`show_box`).

use std::ops::Range;

use super::super::{App, Focus};

impl App {
    /// Whether the cursor stands on a stop of the boxes under the selected block, a paragraph of
    /// a box's words or a thread's fold line: the stop `down` and `up` reached, or the words the
    /// cursor was left on.
    pub(in crate::app) fn on_box_line(&self) -> bool {
        self.open.layout.row(self.cursor.0).is_none() && self.box_block(self.cursor.0) == Some(self.selected)
    }

    /// Whether document row `row` is a row of a stop of the boxes: a paragraph's, or a fold line.
    pub(in crate::app) fn box_stop(&self, row: usize) -> bool {
        self.fold_line(row).is_some() || self.paragraph_at(row).is_some()
    }

    /// The stop of the boxes at document row `row`, as the rows it is drawn on: a thread's fold
    /// line, or the paragraph of a message's words the row shows, whole.
    pub(in crate::app) fn box_unit(&self, row: usize) -> Option<Range<usize>> {
        if self.fold_line(row).is_some() {
            return Some(row..row + 1);
        }
        let here = self.paragraph_at(row)?;
        let same = |row: &usize| self.paragraph_at(*row).as_ref() == Some(&here);
        let first = (0..row).rev().take_while(same).last().unwrap_or(row);
        let last = (row + 1..self.open.layout.total_rows).take_while(same).last().unwrap_or(row);
        Some(first..last + 1)
    }

    /// Whether a stop of the boxes is selected as a block is, with no cursor drawn.
    pub(in crate::app) fn box_line_selected(&self) -> bool {
        self.focus == Focus::Document
            && !self.roam
            && self.selection.is_none()
            && self.pending.is_none()
            && self.on_box_line()
    }

    /// Select the stop of the boxes drawn on document rows `rows`, the cursor on its first word
    /// if it has words, all of it in view: where `down` and `up` leave it, and a click on a fold
    /// line.
    pub(in crate::app) fn stand_on(&mut self, rows: Range<usize>) {
        self.clear_selection();
        self.roam = false;
        self.cursor = (rows.start, self.row_start(rows.start));
        if let Some(block) = self.box_block(rows.start) {
            self.selected = block;
        }
        self.ensure_rows_visible(rows.start, rows.end.saturating_sub(1).max(rows.start));
    }

    /// Bring the box of thread `id` into view, at any depth, as a unit of selection is
    /// (`ensure_rows_visible`): all of it when it fits, else its head first and as much of it as
    /// fits; nothing moves when that is in view already. Whether the flow draws it: a sub-thread
    /// in a folded box is not.
    pub(in crate::app) fn show_box(&mut self, id: &str) -> bool {
        let Some(rows) = self.box_rows(id) else { return false };
        let height = usize::from(self.geometry.doc.height.max(1));
        let last = rows.end.saturating_sub(1).min(rows.start + height.saturating_sub(2));
        self.ensure_rows_visible(rows.start, last.max(rows.start));
        true
    }

    /// The document rows the box of thread `id` is drawn on, at any depth: from its head to its
    /// bottom edge, the rows of the boxes in it included, or its folded line alone.
    fn box_rows(&self, id: &str) -> Option<Range<usize>> {
        let (head, _) = self.find_fold(id)?;
        let in_box = |row: &usize| {
            self.box_row(*row).is_some_and(|(_, drawn)| {
                drawn.thread.as_deref() == Some(id) || drawn.border.iter().any(|(of, _)| of == id)
            })
        };
        let last = (head + 1..self.open.layout.total_rows).take_while(in_box).last().unwrap_or(head);
        Some(head..last + 1)
    }

    /// The columns of the box row at document row `row`, `width` columns wide, that a stop
    /// selected is drawn over: between the innermost box's sides, a fold line's between the
    /// sides of the boxes it is in.
    pub(in crate::app) fn box_columns(&self, row: usize, width: usize) -> Range<usize> {
        let Some((_, drawn)) = self.box_row(row) else { return 0..0 };
        let words = &drawn.words;
        let start = words.iter().position(Option::is_some).unwrap_or(words.len());
        // The sides take as many columns on the right as on the left.
        start..width.saturating_sub(start).max(start)
    }
}
