//! Threads in the document's flow, while the review loop keeps the document's comments. Each
//! comment the journal has, a draft (`Not sent`) included, is drawn as a box right under the
//! block its passage starts in: the passage, its kind and its state at its head, then the
//! comment and each reply with who wrote it. A thread opened on one of those messages is a box
//! inside the box, under the paragraph its passage starts in, at the same indent at every
//! depth, its passage marked in the message's words as the document marks a comment's. One code
//! draws them all, whatever the depth and the kind (`rows`): a blank row above and below each
//! box, one passage not found at the end of the words it was on, marked so. A box open has `▾`
//! first at its head; folded, it is one line, `▸` first, saying what its head says. By default
//! a resolved box folds once nothing under it, at any depth, is open, and every other box is
//! open; any box, whatever its depth, kind and state, folds or opens by hand, and stays so
//! (`fold`): folding resolves nothing and opening reopens nothing. Without the loop nothing is
//! drawn here: a record's replies and resolved state change nothing on screen, as before the
//! loop.
//!
//! Every box's border is grey at rest, at any depth, of any kind and in any state; the box the
//! person is in, the innermost, the current note's, whose bubble the rail lights, is drawn in
//! that bubble's lit colour (`draw::lit`): its kind's, green once resolved. A box holding it
//! stays grey. On a line of the document, no box entered, every note on it is lit, boxes and
//! bubbles alike (`lit_notes`).
//!
//! The layout keeps the boxes' rows free under their blocks (`DocLayout::reserve_under`),
//! so the document's own rows never move inside a block. The rows are rebuilt only when a
//! thread or the width changes; how a thread is drawn in them is `rows`. The boxes lit are
//! lit as the rows are drawn, over the columns each row keeps as its border, so moving from
//! one box to another rebuilds nothing. Each row knows, per column, the message's
//! words it shows, so a passage of them can be selected as the document's words are (`words`),
//! and `down` and `up` stop on their lines as on the blocks (`lines`), and on a thread's fold
//! line: its folded line, or the head of its box.
//!
//! The threads come from the review's view (`journal`), sub-threads, sends and what is new
//! included; one way folds a thread or opens it (`fold`), from `right` (or `confirm`) on its
//! folded line, `left` on its head, a click on either, and `toggle-thread`. What the person types
//! about a box is typed in it, whatever its depth (`field`): a reply at its bottom, a sub-thread
//! under the paragraph its passage starts in, the edit of a draft in place of its words.

mod field;
mod fold;
mod head;
mod journal;
mod lines;
mod rows;
mod words;

use std::collections::HashMap;
use std::ops::Range;

use plannotator_tui_schema::Kind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::draw::lit;
use super::{App, Focus, Mode};
use crate::store::Placed;
use field::Field;
use rows::{BoxRow, Under, spaced};

pub(super) use head::{RESOLVED, resolved_style};
pub(super) use journal::state_marks;
pub(super) use words::Passage;

/// One thread as drawn: the passage it is on, then its messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Thread {
    id: String,
    kind: Kind,
    quote: String,
    /// Where its passage is in the words it is on: the document's source, or, for a sub-thread,
    /// its message's words, where it is marked as the document's passages are, its box under
    /// the paragraph it starts in; `None` when the words no longer hold it.
    range: Option<Range<usize>>,
    resolved: bool,
    /// Drawn folded by default: the review's view's word, else `settled`.
    folded: Option<bool>,
    /// Folded (`true`) or drawn open (`false`) by hand, over its default (`Flow::by_hand`).
    by_hand: Option<bool>,
    /// What the head says after the state: its kind when not a comment, not found, then what
    /// its bubble says too (`journal::state_marks`): what became of its send, live, the round.
    marks: Vec<String>,
    messages: Vec<Message>,
    /// The field the person types in its box, while they do, and where in it (`field`).
    field: Option<Field>,
}

/// The note, or one reply under it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    /// The note's or the reply's id: what a sub-thread on its words is under.
    id: String,
    author: String,
    /// What the line naming the author adds: its round or a draft, what became of a send, new.
    tag: Option<String>,
    body: String,
    /// The person's draft, not sent yet: its words take no sub-thread.
    draft: bool,
    /// Threads opened on this message's words, drawn under it.
    threads: Vec<Thread>,
}

impl Thread {
    /// Whether it is drawn folded: as folded or opened by hand, else by default.
    fn is_folded(&self) -> bool {
        self.by_hand.unwrap_or_else(|| self.folds())
    }

    /// Whether it is drawn folded by default, nothing done by hand.
    fn folds(&self) -> bool {
        self.folded.unwrap_or_else(|| self.settled())
    }

    /// Fold or open by hand the threads `by_hand` names, at any depth, the others as by
    /// default.
    fn by_hand(&mut self, by_hand: &HashMap<String, bool>) {
        self.by_hand = by_hand.get(&self.id).copied();
        for message in &mut self.messages {
            for sub in &mut message.threads {
                sub.by_hand(by_hand);
            }
        }
    }

    /// Resolved, and so is every thread under it at any depth: it is drawn folded.
    fn settled(&self) -> bool {
        self.resolved && self.sub_threads().all(Self::settled)
    }

    fn sub_threads(&self) -> impl Iterator<Item = &Self> {
        self.messages.iter().flat_map(|message| &message.threads)
    }

    /// Whether a reply in it, at any depth, is new: its folded line says so, as the line of its
    /// author does in the box.
    fn holds_new(&self) -> bool {
        self.messages.iter().any(|message| message.tag.as_deref() == Some(journal::NEW))
            || self.sub_threads().any(Self::holds_new)
    }

    /// Its messages and the threads under it, at every depth.
    fn size(&self) -> (usize, usize) {
        self.sub_threads().fold((self.messages.len(), 0), |(messages, threads), sub| {
            let (sub_messages, sub_threads) = sub.size();
            (messages + sub_messages, threads + 1 + sub_threads)
        })
    }
}

/// The threads drawn in the open document and their rows, kept until a thread or the
/// width changes.
#[derive(Debug, Default)]
pub(super) struct Flow {
    width: usize,
    /// Each thread with the block its passage starts in, in document order.
    threads: Vec<(usize, Thread)>,
    /// The rows under each block that has threads, in block order.
    rows: Vec<(usize, Vec<BoxRow>)>,
    /// The messages whose words the rows show, as their columns name them.
    said: Vec<words::Said>,
    /// How many rows that is per block: what the layout keeps free.
    under: Vec<(usize, usize)>,
    /// The threads folded (`true`) or drawn open (`false`) by hand, by id, unlike their default.
    by_hand: HashMap<String, bool>,
}

impl Flow {
    fn new(threads: Vec<(usize, Thread)>, width: usize, by_hand: HashMap<String, bool>) -> Self {
        let mut said = Vec::new();
        let mut rows: Vec<(usize, Vec<BoxRow>)> = Vec::new();
        for group in threads.chunk_by(|(one, _), (other, _)| one == other) {
            let Some((block, _)) = group.first() else { continue };
            // The gap under the block is the blank row below the last box.
            let boxes = group.iter().map(|(_, thread)| Under::Thread(thread));
            let lines = spaced(boxes, width, &mut said, true);
            rows.push((*block, lines));
        }
        let under = rows.iter().map(|(block, lines)| (*block, lines.len())).collect();
        Self { width, threads, rows, said, under, by_hand }
    }
}

impl App {
    /// Match the flow to the open document's comments and width, and the field the person types
    /// in a box, and keep its rows free under their blocks; none without the review loop. When
    /// nothing changed this compares the threads and wraps nothing.
    pub(super) fn sync_threads(&mut self) {
        let mut threads = self.journal_threads().unwrap_or_default();
        let kept = self.kept_at_cursor();
        let typing = self.typing().map(|(target, verb)| (target, self.compose_title(verb)));
        let compose = &self.compose;
        let open = &mut self.open;
        for (_, thread) in &mut threads {
            thread.by_hand(&open.flow.by_hand);
            if let Some((target, title)) = &typing {
                let field = |place, words| Field { place, title: title.clone(), rows: compose.rows(words) };
                thread.typing(target, open.layout.width, &field);
            }
        }
        if threads != open.flow.threads || open.layout.width != open.flow.width {
            let by_hand = std::mem::take(&mut open.flow.by_hand);
            open.flow = Flow::new(threads, open.layout.width, by_hand);
        }
        if open.layout.reserve_under(&open.flow.under) {
            // Rows below a changed box moved, as on a reflow: a selection drawn by rows no
            // longer covers what it did. The selection waiting for an action is a source
            // range, which has not moved: it only follows its first row. A passage of a box's
            // words follows its words, or goes with them, and so does the cursor in them, or on
            // a thread's fold line, its words drawn selected again where they were (`boxes_moved`):
            // a field opening or growing under them leaves them selected.
            let selected = self.selection.take().is_some();
            if let Some(pending) = self.pending.as_mut()
                && let Some(range) = pending.range()
            {
                let block = open.doc.block_containing(range.start);
                let row = block.and_then(|block| open.layout.first_row_in_range(block, range));
                pending.at.0 = row.unwrap_or(pending.at.0);
            }
            self.scroll_by(0);
            self.boxes_moved(kept, selected);
        }
    }

    /// The flow's rows that are in view, on the rows the layout keeps free for them, the
    /// borders of the boxes lit (`lit_notes`) in their colour, with the passages of the
    /// sub-threads, the selection and the cursor over their words, and the line `down` and `up`
    /// selected.
    pub(super) fn draw_threads(&self, buf: &mut Buffer, gutter: Rect, doc: Rect) {
        let view = self.scroll..self.scroll + usize::from(doc.height);
        let lit = self.lit_boxes();
        for (block, rows) in &self.open.flow.rows {
            let Some(rendered) = self.open.layout.blocks.get(*block) else { continue };
            let top = rendered.first_row + rendered.rows.len();
            for row in view.start.max(top)..view.end.min(top + rows.len()) {
                let Some(drawn) = rows.get(row - top) else { continue };
                let y = doc.y + (row - self.scroll) as u16;
                buf.set_line(doc.x, y, &drawn.line, doc.width);
                light(buf, Rect { y, height: 1, ..doc }, &drawn.border, &lit);
                self.mark_line(buf, (gutter, doc), (row, y));
                self.mark_passages(buf, doc, y, &drawn.words);
                self.mark_words(buf, doc, (row, y), &drawn.words);
            }
        }
    }

    /// The notes lit, their boxes here and their bubbles in the rail alike, so the two never
    /// differ: in the rail, or in a box at any depth, the current note's alone (`current_note`),
    /// the innermost box's, not the threads on the line of words the cursor is on, and while a
    /// reply or an edit is typed in a box (`field`), that box's; on a line of the document, the
    /// selected block, every note on it, the current note among them, as the rail lit them
    /// before the boxes: each comment whose passage starts in it, the sub-threads under them,
    /// and under the last block the comments whose words are no longer found.
    pub(super) fn lit_notes(&self) -> Vec<String> {
        if let Mode::Reply(id) | Mode::Edit(id) = &self.mode {
            return vec![id.clone()];
        }
        if self.focus == Focus::Rail || self.thread_at_cursor().is_some() {
            return self.current_note().into_iter().collect();
        }
        let doc = &self.open.doc;
        let on_line = |placed: &&Placed<'_>| doc.block_containing(placed.range.start) == Some(self.selected);
        self.open.store.notes().iter().filter(on_line).map(|placed| placed.annotation.id.clone()).collect()
    }

    /// The boxes lit (`lit_notes`), each with the colour its border takes: its bubble's lit.
    fn lit_boxes(&self) -> Vec<(String, Style)> {
        let lit_box = |id: String| {
            let thread = self.open.flow.thread(&id)?;
            let style = lit(thread.kind, thread.resolved);
            Some((id, style))
        };
        self.lit_notes().into_iter().filter_map(lit_box).collect()
    }
}

/// The columns of a row, drawn on `line`, that are a lit box's border, in its colour (`lit`,
/// each thread with its).
fn light(buf: &mut Buffer, line: Rect, border: &[(String, Range<usize>)], lit: &[(String, Style)]) {
    let width = usize::from(line.width);
    for (of, columns) in border {
        let Some((_, style)) = lit.iter().find(|(id, _)| id == of) else { continue };
        let (start, end) = (columns.start.min(width), columns.end.min(width));
        if end > start {
            let lit = Rect { x: line.x + start as u16, width: (end - start) as u16, ..line };
            buf.set_style(lit, *style);
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use plannotator_tui_schema::Kind;
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

    use super::super::review_test_support::{answer_on_disk, draw, file_app, reply_json};

    /// A record another client wrote a reply and a resolution into draws no thread without the
    /// review loop: the document is laid out as before the loop, and `toggle-thread` is silent.
    #[test]
    fn without_the_review_loop_a_records_replies_draw_no_thread() {
        let (root, mut app, _) = file_app("threads-none");
        app.add_quote_annotation("two", Kind::Comment, "Why two?".into()).expect("note");
        let reply = serde_json::json!([reply_json("Two keeps it short.", "claude")]);
        answer_on_disk(&mut app, 0, reply, "resolved");
        // Under 80 columns the rail is gone: what is left is the document's own column.
        let screen = draw(&mut app, 70, 20);
        let rows: Vec<&str> = screen.lines().collect();
        // A row's words, past the gutter's mark.
        let words = |row: &str| row.trim_start_matches(['\u{258d}', ' ']).trim_end().to_owned();
        let row = |needle: &str| rows.iter().position(|row| words(row) == needle).expect(needle);
        assert_eq!(row("three"), row("two") + 2, "the gap alone, no box: {screen}");
        for gone in ["\u{256d}", "\u{25b8} \u{2713}", "Two keeps it short.", "Resolved"] {
            assert!(!screen.contains(gone), "{gone}: {screen}");
        }
        app.select_block(2);
        app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('z')))).expect("z");
        assert_eq!(app.status, None, "nothing to fold, and nothing said");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
