//! The words of the messages in the threads' boxes, selected as the document's are. Each row
//! of the flow knows, per column, the message whose words it shows and their byte there
//! (`Spot`), so the selection the document uses, by mouse or by keyboard, works in a box too,
//! and a passage selected in one message's words takes a sub-thread on that message, as one
//! selected in the thread opened as a document does. Only while the review loop keeps the
//! document's comments: the journal is what takes a sub-thread.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::super::selection::Selection;
use super::super::{App, Focus};
use super::rows::BoxRow;
use crate::journal::passage;
use crate::keys::Action;
use crate::theme::palette;

/// A message whose words a box draws.
#[derive(Debug)]
pub(super) struct Said {
    /// The note's or the reply's id.
    id: String,
    words: String,
    /// The person's draft: its words take no sub-thread until it is sent.
    draft: bool,
}

impl Said {
    pub(super) fn of(id: &str, words: &str, draft: bool) -> Self {
        Self { id: id.to_owned(), words: words.to_owned(), draft }
    }
}

/// A column of a box that shows a message's words: the message, by its place in the flow's
/// `said`, and the byte of its words shown there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Spot {
    message: usize,
    byte: usize,
}

impl Spot {
    pub(super) fn at(message: usize, byte: usize) -> Self {
        Self { message, byte }
    }
}

/// A passage selected in a message's words in its box: what a sub-thread on it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct Passage {
    /// The message: a comment's or a reply's id.
    pub(in crate::app) parent: String,
    /// The words selected, as the message has them.
    pub(in crate::app) quote: String,
    /// Where they start in the message's words, as Plannotator counts it (`passage::offset_in`).
    pub(in crate::app) offset: Option<u64>,
}

impl App {
    /// Whether the boxes' words take a selection: the review loop keeps the open document's
    /// comments. Otherwise the cursor and the selection step over the boxes, as they always did.
    fn box_words_selectable(&self) -> bool {
        self.review_loop_runs() && self.open.store.is_journal()
    }

    /// The block whose boxes are drawn at document row `row`, and the row of them there.
    fn box_row(&self, row: usize) -> Option<(usize, &BoxRow)> {
        let layout = &self.open.layout;
        self.open.flow.rows.iter().find_map(|(block, rows)| {
            let rendered = layout.blocks.get(*block)?;
            let top = rendered.first_row + rendered.rows.len();
            rows.get(row.checked_sub(top)?).map(|drawn| (*block, drawn))
        })
    }

    /// The words of the box row at document row `row`, per column, when it shows some and the
    /// boxes' words take a selection.
    fn box_words(&self, row: usize) -> Option<&[Option<Spot>]> {
        if !self.box_words_selectable() {
            return None;
        }
        let (_, drawn) = self.box_row(row)?;
        drawn.words.iter().any(Option::is_some).then_some(drawn.words.as_slice())
    }

    /// The block a box's words at document row `row` are under: the block whose thread they
    /// are, so the thread keys act on it.
    pub(in crate::app) fn box_block(&self, row: usize) -> Option<usize> {
        self.box_words(row)?;
        self.box_row(row).map(|(block, _)| block)
    }

    /// The nearest row from `row` on, going down (or up), that the cursor can stand on: the
    /// document's text, and the words in its boxes while they take a selection.
    pub(in crate::app) fn text_row_from(&self, row: usize, down: bool) -> Option<usize> {
        if !self.box_words_selectable() {
            return self.open.layout.text_row_from(row, down);
        }
        let shows = |row: &usize| self.open.layout.row(*row).is_some() || self.box_words(*row).is_some();
        if down { (row..self.open.layout.total_rows).find(shows) } else { (0..=row).rev().find(shows) }
    }

    /// The columns of document row `row` the cursor can reach: the document's cells, or a box's
    /// up to the last of its words.
    pub(in crate::app) fn row_width(&self, row: usize) -> usize {
        match self.open.layout.row(row) {
            Some(drawn) => drawn.cells.len(),
            None => self.box_words(row).map_or(0, <[_]>::len),
        }
    }

    /// The text of document row `row` as drawn, for the word motions: the document's, or a
    /// box's row with its words.
    pub(in crate::app) fn row_text(&self, row: usize) -> Option<String> {
        if let Some(drawn) = self.open.layout.row(row) {
            return Some(drawn.line.to_string());
        }
        self.box_words(row)?;
        self.box_row(row).map(|(_, drawn)| drawn.line.to_string())
    }

    /// The passage `selection` covers in a box's words: `None` when it covers none, `Err` with
    /// why when they take no sub-thread (words of two messages, a draft's words).
    pub(in crate::app) fn box_passage(&self, selection: &Selection) -> Option<Result<Passage, String>> {
        let ((top, _), (bottom, _)) = selection.ordered();
        let mut spots = Vec::new();
        for row in top..=bottom {
            let Some(words) = self.box_words(row) else { continue };
            let columns = selection.columns_on(row, words.len()).unwrap_or(0..0);
            spots.extend(words.iter().skip(columns.start).take(columns.len()).flatten().copied());
        }
        let message = spots.first()?.message;
        if spots.iter().any(|spot| spot.message != message) {
            return Some(Err("a sub-thread is on one message's words: select within one".to_owned()));
        }
        let said = self.open.flow.said.get(message)?;
        if said.draft {
            let send = self.keys.hint(Action::Send);
            return Some(Err(format!("a draft takes no sub-thread: {send} sends it first")));
        }
        let first = spots.iter().map(|spot| spot.byte).min()?;
        let last = spots.iter().map(|spot| spot.byte).max()?;
        let end =
            last + said.words.get(last..).and_then(|rest| rest.chars().next()).map_or(1, char::len_utf8);
        let selected = said.words.get(first..end)?;
        let quote = selected.trim();
        if quote.is_empty() {
            return None;
        }
        let start = first + (selected.len() - selected.trim_start().len());
        let chars: Vec<(char, usize)> = said.words.char_indices().map(|(at, ch)| (ch, at)).collect();
        let nth = passage::occurrence(&chars, quote, start);
        Some(Ok(Passage {
            parent: said.id.clone(),
            quote: quote.to_owned(),
            offset: passage::offset_in(&said.words, quote, nth),
        }))
    }

    /// The selection and the keyboard cursor over a box's row, document row `row` on screen
    /// row `y`: the selection on the words alone, not the box around them.
    pub(super) fn mark_words(
        &self,
        buf: &mut Buffer,
        doc: Rect,
        (row, y): (usize, u16),
        words: &[Option<Spot>],
    ) {
        if !self.box_words_selectable() || words.iter().all(Option::is_none) {
            return;
        }
        let width = usize::from(doc.width);
        if let Some(columns) = self.selection.and_then(|selection| selection.columns_on(row, words.len())) {
            let marked =
                columns.filter(|column| *column < width && words.get(*column).is_some_and(Option::is_some));
            for column in marked {
                buf.set_style(Rect { x: doc.x + column as u16, y, width: 1, height: 1 }, palette().selection);
            }
        }
        let keyboard = self.roam || self.selection.is_some_and(|selection| selection.dragging);
        if self.focus == Focus::Document && keyboard && row == self.cursor.0 {
            let x = doc.x + self.cursor.1.min(width.saturating_sub(1)) as u16;
            buf.set_style(Rect { x, y, width: 1, height: 1 }, palette().cursor);
        }
    }
}
