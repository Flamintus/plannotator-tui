//! The lines of the boxes' words as stops of `down` and `up`, as the blocks are: on the way
//! from a block to the next, each line of a comment, a reply or a sub-thread, at any depth, is
//! selected in turn and drawn as the selected block is; the box's other rows (its head, its
//! sides, the authors, the blank rows) are stepped over as the gaps between blocks are. On such
//! a line `comment` takes a sub-thread on its words, as a block takes a comment, `visual`
//! selects from its first word, and the thread keys act on its box. Only while the boxes'
//! words take a selection: otherwise `down` and `up` go from block to block, as they always
//! did.

use anyhow::Result;
use plannotator_tui_schema::Kind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use super::super::selection::{Pending, Selection};
use super::super::{App, Focus, Selected};
use crate::theme::palette;

impl App {
    /// Whether the cursor stands on a line of a box's words under the selected block: the
    /// stop `down` and `up` reached, or the line the cursor was left on.
    pub(in crate::app) fn on_box_line(&self) -> bool {
        self.cursor_in_box_words() && self.box_block(self.cursor.0) == Some(self.selected)
    }

    /// Whether a line of a box's words is selected as a block is, with no cursor drawn: the
    /// block above it is not drawn selected then.
    pub(in crate::app) fn box_line_selected(&self) -> bool {
        self.focus == Focus::Document
            && !self.roam
            && self.selection.is_none()
            && self.pending.is_none()
            && self.on_box_line()
    }

    /// `down` (or `up`) with no cursor drawn: the next block that way, unless a line of a box's
    /// words comes first; from a line, the next line, or the block past the boxes.
    pub(in crate::app) fn step(&mut self, down: bool) {
        let next_block = if down { self.selected + 1 } else { self.selected.saturating_sub(1) };
        let on_line = self.on_box_line();
        let from = if on_line {
            Some(self.cursor.0)
        } else {
            let block = self.open.layout.blocks.get(self.selected);
            block.map(|block| {
                if down { block.first_row + block.rows.len().saturating_sub(1) } else { block.first_row }
            })
        };
        let next = from.filter(|_| self.box_words_selectable()).and_then(|from| {
            let row = if down { Some(from + 1) } else { from.checked_sub(1) };
            row.and_then(|row| self.text_row_from(row, down))
        });
        match next {
            Some(row) if self.open.layout.row(row).is_none() => self.stand_on_line(row),
            _ if !on_line => self.select_block(next_block),
            Some(row) => {
                if let Some(block) = self.open.layout.block_at_row(row) {
                    self.select_block(block);
                }
            }
            // The last line of the boxes under the last block: nothing further down.
            None => {}
        }
    }

    /// Select the line of a box's words at document row `row`, the cursor on its first word.
    fn stand_on_line(&mut self, row: usize) {
        self.clear_selection();
        self.roam = false;
        self.cursor = (row, self.row_start(row));
        if let Some(block) = self.box_block(row) {
            self.selected = block;
        }
        self.ensure_cursor_visible();
    }

    /// `comment` with the cursor on a line of a box's words and nothing selected: a sub-thread
    /// on the line's words, as a block takes a comment; the footer says why there is none (a
    /// draft's words).
    pub(in crate::app) fn sub_thread_on_line(&mut self) -> Result<()> {
        let row = self.cursor.0;
        let Some(words) = self.box_words(row) else { return Ok(()) };
        let first = words.iter().position(Option::is_some).unwrap_or(0);
        let last = words.iter().rposition(Option::is_some).unwrap_or(first);
        let line = Selection::finished((row, first), (row, last));
        match self.box_passage(&line) {
            Some(Ok(passage)) => {
                self.selection = Some(line);
                self.status = None;
                self.pending = Some(Pending { on: Selected::Passage(passage), at: (row, first) });
                self.act(Kind::Comment)?;
            }
            Some(Err(why)) => self.status = Some(why),
            None => {}
        }
        Ok(())
    }

    /// The selected line of a box's words, document row `row` on screen row `y`, drawn as the
    /// selected block is: its background between the innermost box's sides, the gutter's mark.
    pub(super) fn mark_line(&self, buf: &mut Buffer, (gutter, doc): (Rect, Rect), (row, y): (usize, u16)) {
        if !self.box_line_selected() || row != self.cursor.0 {
            return;
        }
        let Some(words) = self.box_words(row) else { return };
        let start = words.iter().position(Option::is_some).unwrap_or(0);
        let width = usize::from(doc.width);
        // The sides take as many columns on the right as on the left.
        let end = width.saturating_sub(start).max(start);
        let line = Rect { x: doc.x + start as u16, y, width: (end - start) as u16, height: 1 };
        buf.set_style(line, Style::new().bg(palette().block_bg));
        buf.set_span(gutter.x, y, &Span::styled("\u{258d}", Style::new().fg(palette().accent)), 1);
    }
}
