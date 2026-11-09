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
//! its state's colour, the last word of its head, and so is its bubble (`head::lit_border`). A
//! box holding it stays grey. On a line of the document, no box entered, every comment on it is
//! lit, not the sub-threads in them, boxes and bubbles alike (`lit_notes`). A bubble says what
//! the head of its box says, as its title, and what its box holds, under its words (`bubble`).
//!
//! The layout keeps the boxes' rows free under their blocks (`DocLayout::reserve_under`),
//! so the document's own rows never move inside a block. The rows are rebuilt only when a
//! thread or the width changes; how a thread is drawn in them is `rows`. The boxes lit are
//! lit as the rows are drawn, over the columns each row keeps as its border, so moving from
//! one box to another rebuilds nothing. Each row knows, per column, the message's
//! words it shows, so a passage of them can be selected as the document's words are (`words`),
//! and `down` and `up` stop on their paragraphs as on the blocks, the same unit of selection
//! (`lines`, `unit`), and on a thread's fold line: its folded line, or the head of its box.
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
use ratatui::text::Line;

use super::{App, Focus, Mode};
use crate::store::Placed;
use field::Field;
use head::{DRAFT, Mark, NOT_SAVED, lit_border, state_of};
use rows::{BoxRow, Under, spaced};

pub(super) use head::{GREY, LIVE, mark_style};
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
    /// What its head says of it, its folded line and its bubble's title alike (`head::Mark`,
    /// `journal::marks`): not found, Live, its round, its state.
    marks: Vec<Mark>,
    /// The images attached to its comment, which its folded line and its bubble count.
    images: usize,
    messages: Vec<Message>,
    /// The field the person types in its box, while they do, and where in it (`field`).
    field: Option<Field>,
    /// A comment on the whole document: its box above the first block, as the web's is above
    /// the title.
    top: bool,
}

/// The note, or one reply under it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    /// The note's or the reply's id: what a sub-thread on its words is under.
    id: String,
    author: String,
    /// What the line naming the author adds: a message of the person's, the state it is still
    /// in (`Draft`, `Sent`); an agent's reply, `New`.
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
        self.messages.iter().any(|message| message.tag.as_deref() == Some(head::NEW))
            || self.sub_threads().any(Self::holds_new)
    }

    /// What its folded line and its bubble's title say of it (`head::Mark`): its head's marks,
    /// and `New` while a reply in its box, at any depth, is.
    fn marks_and_new(&self) -> Vec<Mark> {
        let mut marks = self.marks.clone();
        marks.extend(self.holds_new().then_some(Mark::New));
        marks
    }

    /// What its box holds, as its folded line and its bubble count it (`head::counts`): its
    /// own messages, the threads under it at every depth, its comment's images.
    fn counts(&self) -> Vec<String> {
        head::counts(self.messages.len(), self.threads_under(), self.images)
    }

    /// How many threads are under it, at every depth.
    fn threads_under(&self) -> usize {
        self.sub_threads().map(|sub| 1 + sub.threads_under()).sum()
    }
}

/// The threads drawn in the open document and their rows, kept until a thread or the
/// width changes.
#[derive(Debug, Default)]
pub(super) struct Flow {
    width: usize,
    /// Each thread with the block its passage starts in, in document order.
    threads: Vec<(usize, Thread)>,
    /// The rows above the first block: the comments on the whole document.
    top: Vec<BoxRow>,
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
        // The comments on the whole document first, above the first block: a blank row above
        // each and one below the last, before the block.
        let general = threads.iter().filter(|(_, thread)| thread.top).map(|(_, thread)| Under::Thread(thread));
        let top = spaced(general, width, &mut said, false);
        let under_blocks: Vec<(usize, Thread)> = threads.iter().filter(|(_, thread)| !thread.top).cloned().collect();
        let mut rows: Vec<(usize, Vec<BoxRow>)> = Vec::new();
        for group in under_blocks.chunk_by(|(one, _), (other, _)| one == other) {
            let Some((block, _)) = group.first() else { continue };
            // The gap under the block is the blank row below the last box.
            let boxes = group.iter().map(|(_, thread)| Under::Thread(thread));
            let lines = spaced(boxes, width, &mut said, true);
            rows.push((*block, lines));
        }
        let under = rows.iter().map(|(block, lines)| (*block, lines.len())).collect();
        Self { width, threads, top, rows, said, under, by_hand }
    }
}

impl App {
    /// Each group of boxes as drawn: the block it belongs to, the document row it starts on and
    /// its rows. The comments on the whole document first, at the top, belonging to the first
    /// block (`k` from it goes up into them); then each block's, right under it.
    pub(in crate::app::threads) fn box_groups(&self) -> impl Iterator<Item = (usize, usize, &Vec<BoxRow>)> {
        let flow = &self.open.flow;
        let layout = &self.open.layout;
        let top = (!flow.top.is_empty()).then_some((0, 0, &flow.top));
        let under = flow.rows.iter().filter_map(move |(block, rows)| {
            let rendered = layout.blocks.get(*block)?;
            Some((*block, rendered.first_row + rendered.rows.len(), rows))
        });
        top.into_iter().chain(under)
    }
}

impl App {
    /// Match the flow to the open document's comments and width, and the field the person types
    /// in a box, and keep its rows free under their blocks; none without the review loop. When
    /// nothing changed this compares the threads and wraps nothing.
    pub(super) fn sync_threads(&mut self) {
        let mut threads = self.journal_threads().unwrap_or_default();
        let kept = self.kept_at_cursor();
        let typing = self.typing().map(|target| (target, self.compose_titles()));
        let compose = &self.compose;
        let open = &mut self.open;
        for (_, thread) in &mut threads {
            thread.by_hand(&open.flow.by_hand);
            if let Some((target, (title, keys))) = &typing {
                let field = |place, words| Field {
                    place,
                    title: title.clone(),
                    keys: keys.clone(),
                    rows: compose.rows(words),
                };
                thread.typing(target, open.layout.width, &field);
            }
        }
        if threads != open.flow.threads || open.layout.width != open.flow.width {
            let by_hand = std::mem::take(&mut open.flow.by_hand);
            open.flow = Flow::new(threads, open.layout.width, by_hand);
        }
        let top_moved = open.layout.reserve_top(open.flow.top.len());
        if open.layout.reserve_under(&open.flow.under) || top_moved {
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
    /// sub-threads, the selection and the cursor over their words, and the stop `down` and `up`
    /// selected drawn as a block is (`unit`).
    pub(super) fn draw_threads(&self, buf: &mut Buffer, gutter: Rect, doc: Rect) {
        let view = self.scroll..self.scroll + usize::from(doc.height);
        let lit = self.lit_boxes();
        let selected = self.unit_rows(&self.selected_unit());
        for (_, top, rows) in self.box_groups() {
            for row in view.start.max(top)..view.end.min(top + rows.len()) {
                let Some(drawn) = rows.get(row - top) else { continue };
                let y = doc.y + (row - self.scroll) as u16;
                buf.set_line(doc.x, y, &drawn.line, doc.width);
                light(buf, Rect { y, height: 1, ..doc }, &drawn.border, &lit);
                self.mark_selected(buf, (gutter, doc), (row, y), &selected);
                self.mark_passages(buf, doc, y, &drawn.words);
                self.mark_words(buf, doc, (row, y), &drawn.words);
            }
        }
    }

    /// The notes lit, their boxes here and their bubbles in the rail alike, so the two never
    /// differ: in the rail, or in a box at any depth, the current note's alone (`current_note`),
    /// the innermost box's, not the threads on the line of words the cursor is on, and while a
    /// reply or an edit is typed in a box (`field`), that box's; on a line of the document, the
    /// selected block, every comment on it: each whose passage starts in it, and under the last
    /// block the comments whose words are no longer found, not the sub-threads under them,
    /// which are on a message's words, not on the line.
    pub(super) fn lit_notes(&self) -> Vec<String> {
        if let Mode::Reply(id) | Mode::Edit(id) = &self.mode {
            return vec![id.clone()];
        }
        if self.focus == Focus::Rail || self.thread_at_cursor().is_some() {
            return self.current_note().into_iter().collect();
        }
        let (doc, store) = (&self.open.doc, &self.open.store);
        let on_line = |placed: &&Placed<'_>| {
            doc.block_containing(placed.range.start) == Some(self.selected)
                && !store.on_passage(&placed.annotation.id)
        };
        store.notes().iter().filter(on_line).map(|placed| placed.annotation.id.clone()).collect()
    }

    /// The boxes lit (`lit_notes`), each with the colour its border takes: its state's.
    fn lit_boxes(&self) -> Vec<(String, Style)> {
        let lit_box = |id: String| {
            let thread = self.open.flow.thread(&id)?;
            let style = lit_border(state_of(&thread.marks));
            Some((id, style))
        };
        self.lit_notes().into_iter().filter_map(lit_box).collect()
    }

    /// What the bubble of note `id` says of it, as its folded line does (`Thread::marks_and_new`);
    /// a note the flow does not draw yet, which waits for a send (`Store::journal_waits`), is a
    /// draft; and its state is `Not saved` while the journal has not taken its last write.
    fn bubble_marks(&self, id: &str) -> Vec<Mark> {
        let waits = || self.open.store.journal_waits(id).unwrap_or(false);
        let unseen = || waits().then_some(Mark::State(DRAFT)).into_iter().collect();
        let mut marks = self.open.flow.thread(id).map_or_else(unseen, Thread::marks_and_new);
        if self.open.store.journal_unsaved(id) {
            marks.retain(|mark| !matches!(mark, Mark::State(_)));
            marks.push(Mark::State(NOT_SAVED));
        }
        marks
    }

    /// The colour the bubble of note `id` takes lit: its state's, as its box's.
    pub(super) fn lit_bubble(&self, id: &str) -> Style {
        lit_border(state_of(&self.bubble_marks(id)))
    }

    /// The top edge of note `placed`'s bubble, `width` columns, corners and all, drawn in
    /// `border`: what the head of its box says (`head::title`).
    pub(super) fn bubble_title(&self, placed: &Placed<'_>, width: usize, border: Style) -> Line<'static> {
        let id = &placed.annotation.id;
        let marks = self.bubble_marks(id);
        match self.open.flow.thread(id) {
            Some(thread) => head::title(thread.kind, &thread.quote, &marks, width, border),
            None => head::title(placed.kind(), placed.annotation.anchor.rendered(), &marks, width, border),
        }
    }

    /// What the box of note `placed` holds, as its bubble counts it under its words
    /// (`head::counts`); a note the flow does not draw yet is one message, with its images.
    pub(super) fn held(&self, placed: &Placed<'_>) -> Vec<String> {
        match self.open.flow.thread(&placed.annotation.id) {
            Some(thread) => thread.counts(),
            None => head::counts(1, 0, placed.annotation.plannotator_tui.images.len()),
        }
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
        app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('f')))).expect("f");
        assert_eq!(app.status, None, "nothing to fold, and nothing said");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
