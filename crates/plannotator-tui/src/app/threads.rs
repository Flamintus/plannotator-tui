//! Threads in the document's flow, while the review loop keeps the document's comments. A
//! comment someone answered, or resolved, is drawn as a box right under the block its passage
//! starts in: the passage, its kind and its state at its head, then the comment and each reply
//! with who wrote it. A thread opened on one of those messages is a box inside the box, under
//! the line its passage starts on, at the same indent at every depth, its passage marked in the
//! message's words as the document marks a comment's. One code draws them all, whatever the
//! depth and the kind (`rows`): a blank row above and below each box, one passage not found
//! at the end of the words it was on, marked so. A resolved box folds to one line once nothing
//! under it, at any depth, is open; until then it stays open, marked resolved. A draft no one
//! has answered stays a bubble in the rail only. Without the loop nothing is drawn here: a
//! record's replies and resolved state change nothing on screen, as before the loop.
//!
//! The layout keeps the boxes' rows free under their blocks (`DocLayout::reserve_under`),
//! so the document's own rows never move inside a block. The rows are rebuilt only when a
//! thread or the width changes; how a thread is drawn in them is `rows`. Each row knows, per
//! column, the message's words it shows, so a passage of them can be selected as the
//! document's words are (`words`), and `down` and `up` stop on their lines as on the blocks
//! (`lines`).
//!
//! The threads come from the review's view (`journal`), sub-threads, sends and what is new
//! included; `toggle-thread` draws a folded thread open, or folds it again (`fold`).

mod fold;
mod head;
mod journal;
mod lines;
mod rows;
mod words;

use std::collections::HashSet;
use std::ops::Range;

use plannotator_tui_schema::Kind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::App;
use rows::{BoxRow, spaced};

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
    /// the line it starts on; `None` when the words no longer hold it.
    range: Option<Range<usize>>,
    resolved: bool,
    /// Drawn folded: the review's view's word, else `settled`; open when unfolded by hand.
    folded: Option<bool>,
    /// What the head says after the state: its kind when not a comment, not found, then what
    /// its bubble says too (`journal::state_marks`): what became of its send, live, the round.
    marks: Vec<String>,
    messages: Vec<Message>,
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
    /// Whether it is drawn folded.
    fn is_folded(&self) -> bool {
        self.folded.unwrap_or_else(|| self.settled())
    }

    /// Draw open the threads `unfolded` names, at any depth.
    fn unfold(&mut self, unfolded: &HashSet<String>) {
        if unfolded.contains(&self.id) {
            self.folded = Some(false);
        }
        for message in &mut self.messages {
            for sub in &mut message.threads {
                sub.unfold(unfolded);
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
    /// Folded threads drawn open by hand, by id.
    unfolded: HashSet<String>,
}

impl Flow {
    fn new(threads: Vec<(usize, Thread)>, width: usize, unfolded: HashSet<String>) -> Self {
        let mut said = Vec::new();
        let mut rows: Vec<(usize, Vec<BoxRow>)> = Vec::new();
        for group in threads.chunk_by(|(one, _), (other, _)| one == other) {
            let Some((block, _)) = group.first() else { continue };
            // The gap under the block is the blank row below the last box.
            let lines = spaced(group.iter().map(|(_, thread)| thread), width, &mut said, true);
            rows.push((*block, lines));
        }
        let under = rows.iter().map(|(block, lines)| (*block, lines.len())).collect();
        Self { width, threads, rows, said, under, unfolded }
    }
}

impl App {
    /// Match the flow to the open document's comments and width, and keep its rows free under
    /// their blocks; none without the review loop. When nothing changed this compares the
    /// threads and wraps nothing.
    pub(super) fn sync_threads(&mut self) {
        let mut threads = self.journal_threads().unwrap_or_default();
        let kept = self.words_at_cursor();
        let open = &mut self.open;
        for (_, thread) in &mut threads {
            thread.unfold(&open.flow.unfolded);
        }
        if threads != open.flow.threads || open.layout.width != open.flow.width {
            let unfolded = std::mem::take(&mut open.flow.unfolded);
            open.flow = Flow::new(threads, open.layout.width, unfolded);
        }
        if open.layout.reserve_under(&open.flow.under) {
            // Rows below a changed box moved, as on a reflow: a selection drawn by rows no
            // longer covers what it did. The selection waiting for an action is a source
            // range, which has not moved: it only follows its first row. A passage of a box's
            // words follows its words, or goes with them, and so does the cursor in them
            // (`boxes_moved`).
            self.selection = None;
            if let Some(pending) = self.pending.as_mut()
                && let Some(range) = pending.range()
            {
                let block = open.doc.block_containing(range.start);
                let row = block.and_then(|block| open.layout.first_row_in_range(block, range));
                pending.at.0 = row.unwrap_or(pending.at.0);
            }
            self.scroll_by(0);
            self.boxes_moved(kept);
        }
    }

    /// The flow's rows that are in view, on the rows the layout keeps free for them, with the
    /// passages of the sub-threads, the selection and the cursor over their words, and the
    /// line `down` and `up` selected.
    pub(super) fn draw_threads(&self, buf: &mut Buffer, gutter: Rect, doc: Rect) {
        let view = self.scroll..self.scroll + usize::from(doc.height);
        for (block, rows) in &self.open.flow.rows {
            let Some(rendered) = self.open.layout.blocks.get(*block) else { continue };
            let top = rendered.first_row + rendered.rows.len();
            for row in view.start.max(top)..view.end.min(top + rows.len()) {
                let Some(drawn) = rows.get(row - top) else { continue };
                let y = doc.y + (row - self.scroll) as u16;
                buf.set_line(doc.x, y, &drawn.line, doc.width);
                self.mark_line(buf, (gutter, doc), (row, y));
                self.mark_passages(buf, doc, y, &drawn.words);
                self.mark_words(buf, doc, (row, y), &drawn.words);
            }
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
        for gone in ["\u{256d}", "\u{2576}", "Two keeps it short.", "Resolved"] {
            assert!(!screen.contains(gone), "{gone}: {screen}");
        }
        app.select_block(2);
        app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('z')))).expect("z");
        assert_eq!(app.status, None, "nothing to fold, and nothing said");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
