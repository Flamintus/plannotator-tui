//! The unit of selection, one for the document's text and its boxes: what `down` and `up` stop
//! on and select, what is drawn selected, what `comment` takes whole with nothing selected, and
//! where `move-cursor` and `visual` start. A block of the document; under the review loop, in the
//! boxes under a block, at any depth, a paragraph of a message's words, the one a sub-thread's
//! box goes under (`doc::plain_blocks`), or a thread's fold line, its head or its folded line
//! (`threads::lines`). From one unit `down` (or `up`) goes to the next one that way; the unit
//! selected has the gutter's mark on each of its rows and, while the document has the keys and
//! nothing is selected or waits for its action, its background: the document's width on a
//! block's rows, between the innermost box's sides on a box's. `comment` on a block comments
//! on the block, on a paragraph opens a sub-thread on its words, typed under it (`threads::field`);
//! a fold line takes none. Without the loop the units are the blocks alone, as they always were.

use std::ops::Range;

use anyhow::Result;
use plannotator_tui_schema::Kind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use super::{App, Focus, Pending, Selected};
use crate::theme::palette;

/// A unit of selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unit {
    /// Block `index` of the document.
    Block(usize),
    /// A stop of the boxes, a paragraph of a message's words or a thread's fold line: the
    /// document rows it is drawn on.
    Boxed(Range<usize>),
}

impl App {
    /// The unit selected: the stop of the boxes the cursor stands on, while the document has
    /// the keys, else the selected block.
    pub(super) fn selected_unit(&self) -> Unit {
        let boxed = self.focus == Focus::Document && self.on_box_line();
        boxed.then(|| self.box_unit(self.cursor.0)).flatten().map_or(Unit::Block(self.selected), Unit::Boxed)
    }

    /// The document rows `unit` is drawn on.
    pub(super) fn unit_rows(&self, unit: &Unit) -> Range<usize> {
        match unit {
            Unit::Block(block) => self
                .open
                .layout
                .blocks
                .get(*block)
                .map_or(0..0, |block| block.first_row..block.first_row + block.rows.len()),
            Unit::Boxed(rows) => rows.clone(),
        }
    }

    /// The unit drawn at document row `row`, if any: the block there, or the stop of the boxes.
    fn unit_at(&self, row: usize) -> Option<Unit> {
        match self.open.layout.block_at_row(row) {
            Some(block) => Some(Unit::Block(block)),
            None => self.box_unit(row).map(Unit::Boxed),
        }
    }

    /// Select `unit`: a block as `select_block` does, a stop of the boxes with the cursor on its
    /// first word, if it has words.
    fn select_unit(&mut self, unit: Unit) {
        match unit {
            Unit::Block(block) => self.select_block(block),
            Unit::Boxed(rows) => self.stand_on(rows),
        }
    }

    /// `down` (or `up`) with no cursor drawn: the next unit that way. From a block with no stop
    /// of the boxes before the next block, the next block, as it always was; at either end,
    /// nothing further.
    pub(super) fn step(&mut self, down: bool) {
        let unit = self.selected_unit();
        let rows = self.unit_rows(&unit);
        let past = if down { Some(rows.end) } else { rows.start.checked_sub(1) };
        let next = past
            .filter(|_| self.box_words_selectable())
            .and_then(|row| self.row_from(row, down, |row| self.box_stop(row)));
        let no_box_before = next.is_none_or(|row| self.open.layout.row(row).is_some());
        match unit {
            Unit::Block(block) if no_box_before => {
                self.select_block(if down { block + 1 } else { block.saturating_sub(1) });
            }
            Unit::Block(_) | Unit::Boxed(_) => {
                if let Some(unit) = next.and_then(|row| self.unit_at(row)) {
                    self.select_unit(unit);
                }
            }
        }
    }

    /// Document row `row` on screen row `y` drawn selected when it is one of the selected unit's
    /// `rows`: the gutter's mark, and, while the document has the keys and nothing is selected
    /// or waits for its action, the background, the document's width on a block's row, between
    /// the innermost box's sides on a box's (`box_columns`). Whether it is.
    pub(super) fn mark_selected(
        &self,
        buf: &mut Buffer,
        (gutter, doc): (Rect, Rect),
        (row, y): (usize, u16),
        rows: &Range<usize>,
    ) -> bool {
        if !rows.contains(&row) {
            return false;
        }
        if self.focus == Focus::Document && self.selection.is_none() && self.pending.is_none() {
            let width = usize::from(doc.width);
            let columns =
                if self.open.layout.row(row).is_some() { 0..width } else { self.box_columns(row, width) };
            let line = Rect { x: doc.x + columns.start as u16, y, width: columns.len() as u16, height: 1 };
            buf.set_style(line, Style::new().bg(palette().block_bg));
        }
        buf.set_span(gutter.x, y, &Span::styled("\u{258d}", Style::new().fg(palette().accent)), 1);
        true
    }

    /// `comment` (or `confirm`) with nothing selected: a comment on the selected unit whole, the
    /// block's words, or a paragraph's, a sub-thread on them. The keys offer it only on words
    /// that take one, a sent message's (`journal::help`); a fold line takes none.
    pub(super) fn comment_unit(&mut self) -> Result<()> {
        let pending = match self.selected_unit() {
            Unit::Block(block) => {
                let (Some(source), Some(rendered)) =
                    (self.open.doc.blocks.get(block), self.open.layout.blocks.get(block))
                else {
                    return Ok(());
                };
                Pending::source(source.range.clone(), (rendered.first_row, 0))
            }
            Unit::Boxed(rows) => {
                let Some((passage, at)) = self.paragraph_passage(rows.start) else { return Ok(()) };
                Pending { on: Selected::Passage(passage), at }
            }
        };
        self.pending = Some(pending);
        self.act(Kind::Comment)
    }
}
