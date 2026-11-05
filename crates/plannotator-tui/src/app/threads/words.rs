//! The words of the messages in the threads' boxes, selected as the document's are. Each row
//! of the flow knows, per column, the message whose words it shows and their byte there
//! (`Spot`), so the selection the document uses, by mouse or by keyboard, works in a box too,
//! and a passage selected in one message's words takes what the document's words take, a note
//! of any kind, a sub-thread on that message (`App::annotate`), as one selected in the thread
//! opened as a document does. Only while the review loop keeps the document's comments: the
//! journal is what takes a sub-thread.

use std::ops::Range;

use plannotator_tui_schema::Kind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::super::draw::{marked, priority};
use super::super::selection::Selection;
use super::super::{App, Focus, Selected};
use super::rows::BoxRow;
use crate::journal::{self, passage};
use crate::keys::Action;
use crate::theme::palette;

/// A message whose words a box draws.
#[derive(Debug)]
pub(super) struct Said {
    /// The note's or the reply's id.
    pub(super) id: String,
    pub(super) words: String,
    /// The person's draft: its words take no sub-thread until it is sent.
    pub(super) draft: bool,
    /// The bytes of its words each sub-thread on them is on, with the sub-thread's kind: marked
    /// as the document's passages are.
    pub(super) passages: Vec<(Range<usize>, Kind)>,
}

/// A column of a box that shows a message's words: the message, by its place in the flow's
/// `said`, and the byte of its words shown there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Spot {
    pub(super) message: usize,
    pub(super) byte: usize,
}

/// A passage selected in a message's words in its box: what a sub-thread on it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct Passage {
    /// The message, the words selected as it has them, and where they start in its words.
    pub(in crate::app) on: journal::Passage,
    /// The byte of the message's words they start at: where the passage is found again when
    /// the boxes move.
    start: usize,
}

impl App {
    /// Whether the boxes' words take a selection: the review loop keeps the open document's
    /// comments. Otherwise the cursor and the selection step over the boxes, as they always did.
    pub(super) fn box_words_selectable(&self) -> bool {
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
    pub(super) fn box_words(&self, row: usize) -> Option<&[Option<Spot>]> {
        if !self.box_words_selectable() {
            return None;
        }
        let (_, drawn) = self.box_row(row)?;
        drawn.words.iter().any(Option::is_some).then_some(drawn.words.as_slice())
    }

    /// The block a box's words at document row `row` are under: the block whose thread they
    /// are.
    pub(in crate::app) fn box_block(&self, row: usize) -> Option<usize> {
        self.box_words(row)?;
        self.box_row(row).map(|(block, _)| block)
    }

    /// The thread whose box the cursor is in, the innermost one, while the boxes take a
    /// selection: the thread keys act on it rather than on the block's first note.
    pub(in crate::app) fn thread_at_cursor(&self) -> Option<String> {
        if self.focus != Focus::Document || !self.box_words_selectable() {
            return None;
        }
        self.box_row(self.cursor.0).and_then(|(_, drawn)| drawn.thread.clone())
    }

    /// Whether the cursor stands in a box's words: `comment` there wants a passage selected.
    pub(in crate::app) fn cursor_in_box_words(&self) -> bool {
        self.open.layout.row(self.cursor.0).is_none() && self.box_words(self.cursor.0).is_some()
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

    /// The first column of document row `row` the cursor can reach: 0 in the document, a box's
    /// first word column past its sides.
    pub(in crate::app) fn row_start(&self, row: usize) -> usize {
        match self.open.layout.row(row) {
            Some(_) => 0,
            None => self.box_words(row).and_then(|words| words.iter().position(Option::is_some)).unwrap_or(0),
        }
    }

    /// The columns of document row `row` the cursor can reach: the document's cells, or a box's
    /// up to the last of its words.
    pub(in crate::app) fn row_width(&self, row: usize) -> usize {
        match self.open.layout.row(row) {
            Some(drawn) => drawn.cells.len(),
            None => self.box_words(row).map_or(0, <[_]>::len),
        }
    }

    /// The text of document row `row`, for the word motions: the document's as drawn, or a
    /// box's words one character per column, the box's sides blank.
    pub(in crate::app) fn row_text(&self, row: usize) -> Option<String> {
        if let Some(drawn) = self.open.layout.row(row) {
            return Some(drawn.line.to_string());
        }
        let said = &self.open.flow.said;
        let shown = |spot: &Option<Spot>| {
            let spot = (*spot)?;
            let ch = said.get(spot.message)?.words.get(spot.byte..)?.chars().next()?;
            Some(if ch.is_whitespace() { ' ' } else { ch })
        };
        Some(self.box_words(row)?.iter().map(|spot| shown(spot).unwrap_or(' ')).collect())
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
        let on = journal::Passage {
            parent: said.id.clone(),
            quote: quote.to_owned(),
            offset: passage::offset_in(&said.words, quote, nth),
        };
        Some(Ok(Passage { on, start }))
    }

    /// The words the cursor stands on in a box: their message's id and byte, to find them
    /// again once the boxes move.
    pub(super) fn words_at_cursor(&self) -> Option<(String, usize)> {
        let words = self.box_words(self.cursor.0)?;
        let spot =
            words.get(self.cursor.1).copied().flatten().or_else(|| words.iter().flatten().next().copied())?;
        Some((self.open.flow.said.get(spot.message)?.id.clone(), spot.byte))
    }

    /// Where the words of message `id` at `byte` are drawn: their document row and column.
    fn find_words(&self, id: &str, byte: usize) -> Option<(usize, usize)> {
        let said = &self.open.flow.said;
        let here =
            |spot: &Spot| spot.byte == byte && said.get(spot.message).is_some_and(|said| said.id == id);
        let layout = &self.open.layout;
        self.open.flow.rows.iter().find_map(|(block, rows)| {
            let rendered = layout.blocks.get(*block)?;
            let top = rendered.first_row + rendered.rows.len();
            rows.iter().enumerate().find_map(|(index, drawn)| {
                let column = drawn.words.iter().position(|spot| spot.as_ref().is_some_and(here))?;
                Some((top + index, column))
            })
        })
    }

    /// After the boxes moved, while their words take a selection: a passage waiting for its
    /// action follows its words, the cursor with it, or goes when they are no longer drawn (its
    /// thread folded, or gone); a cursor that stood in a box's words (`kept`, before they
    /// moved) follows them too, or, when they are no longer drawn, goes back to the selected
    /// block, so the thread keys never act on a box it is no longer in.
    pub(in crate::app) fn boxes_moved(&mut self, kept: Option<(String, usize)>) {
        if !self.box_words_selectable() {
            return;
        }
        let Some(Selected::Passage(passage)) = self.pending.as_ref().map(|pending| &pending.on) else {
            let in_box = kept.is_some() || self.open.layout.row(self.cursor.0).is_none();
            match kept.and_then(|(id, byte)| self.find_words(&id, byte)) {
                Some(at) => {
                    self.cursor = at;
                    self.selected = self.box_block(at.0).unwrap_or(self.selected);
                }
                None if in_box => {
                    if let Some(block) = self.open.layout.blocks.get(self.selected) {
                        self.cursor = (block.first_row, 0);
                    }
                }
                None => {}
            }
            return;
        };
        let found = self.find_words(&passage.on.parent, passage.start);
        match (found, self.pending.as_mut()) {
            (Some(at), Some(pending)) => {
                pending.at = at;
                self.cursor = at;
            }
            _ => self.pending = None,
        }
    }

    /// The passages of the sub-threads on a box's row's words, on screen row `y`, marked as the
    /// document marks its comments' passages, whatever their state: under the selection and
    /// the cursor, which are drawn over them (`mark_words`).
    pub(super) fn mark_passages(&self, buf: &mut Buffer, doc: Rect, y: u16, words: &[Option<Spot>]) {
        let said = &self.open.flow.said;
        for (column, spot) in words.iter().enumerate().take(usize::from(doc.width)) {
            let Some(spot) = spot else { continue };
            let Some(said) = said.get(spot.message) else { continue };
            let kind = said
                .passages
                .iter()
                .filter(|(range, _)| range.contains(&spot.byte))
                .map(|(_, kind)| *kind)
                .max_by_key(|&kind| priority(kind));
            let Some(kind) = kind else { continue };
            buf.set_style(Rect { x: doc.x + column as u16, y, width: 1, height: 1 }, marked(kind));
        }
    }

    /// The selection and the keyboard cursor over a box's row, document row `row` on screen
    /// row `y`: the selection on the words alone, not the box around them, while it is drawn
    /// or waits as a passage (a selection of the document's words leaves the boxes out).
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
        let passage = self.pending.as_ref().is_some_and(|pending| matches!(pending.on, Selected::Passage(_)));
        let selection = self.selection.filter(|selection| selection.dragging || passage);
        if let Some(columns) = selection.and_then(|selection| selection.columns_on(row, words.len())) {
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
