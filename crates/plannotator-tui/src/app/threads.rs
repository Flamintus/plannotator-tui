//! Threads in the document's flow. A note someone answered, or resolved, is drawn as a box
//! right under the block its passage starts in: the passage at its head, then the note and
//! each reply with who wrote it. A thread opened on one of those messages is a box inside
//! the box, under that message, at the same indent at every depth. A resolved box folds to
//! one line once nothing under it, at any depth, is open; until then it stays open, marked
//! resolved. A note no one has answered stays a bubble in the rail only.
//!
//! The layout keeps the boxes' rows free under their blocks (`DocLayout::reserve_under`),
//! so the document's own rows never move inside a block. The rows are rebuilt only when a
//! thread or the width changes; how a thread is drawn in them is `rows`. Each row knows, per
//! column, the message's words it shows, so a passage of them can be selected as the
//! document's words are (`words`).
//!
//! A document whose comments the review's journal keeps draws them from the review's view
//! (`journal`), sub-threads, sends and what is new included; `toggle-thread` draws a folded
//! thread open, or folds it again (`fold`).

mod fold;
mod journal;
mod rows;
mod words;

use std::collections::HashSet;

use plannotator_tui_schema::{Annotation, Kind, State};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{App, bubble, label};
use rows::{BoxRow, lines_of};

pub(super) use words::Passage;

/// One thread as drawn: the passage it is on, then its messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Thread {
    id: String,
    kind: Kind,
    quote: String,
    resolved: bool,
    /// Drawn folded: the source's word when it has one (the review's view), else `settled`;
    /// open when unfolded by hand.
    folded: Option<bool>,
    /// What the head says after the state: not found, what became of its send, live.
    marks: Vec<String>,
    messages: Vec<Message>,
}

/// The note, or one reply under it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    /// The note's or the reply's id: what a sub-thread on its words is under.
    id: String,
    author: String,
    /// What the line naming the author adds: a draft, what became of a send, new.
    tag: Option<String>,
    body: String,
    /// The person's draft, not sent yet: its words take no sub-thread.
    draft: bool,
    /// Threads opened on this message's words, drawn under it. A record's replies are flat,
    /// so a note read from one has none; a source that keeps such threads fills them.
    threads: Vec<Thread>,
}

impl Thread {
    /// A note as a thread, or `None` for a note no one has answered or resolved.
    pub(super) fn of(annotation: &Annotation) -> Option<Self> {
        let resolved = annotation.state == State::Resolved;
        if annotation.replies.is_empty() && !resolved {
            return None;
        }
        let kind = annotation.anchor.kind();
        // A note without an author is the reviewer's own: local notes carry none. A reply
        // without one is anonymous, as the record defines it (`bubble::author`).
        let author = annotation.author_name.as_deref().or(annotation.author.as_deref()).unwrap_or("You");
        let body = if annotation.body.is_empty() { label(kind) } else { annotation.body.as_str() };
        let note = Message {
            id: annotation.id.clone(),
            author: author.to_owned(),
            tag: None,
            body: body.to_owned(),
            draft: false,
            threads: Vec::new(),
        };
        let replies = annotation.replies.iter().map(|reply| Message {
            id: reply.id.clone(),
            author: bubble::author(reply).to_owned(),
            tag: None,
            body: reply.body.clone(),
            draft: false,
            threads: Vec::new(),
        });
        Some(Self {
            id: annotation.id.clone(),
            kind,
            quote: annotation.anchor.rendered().to_owned(),
            resolved,
            folded: None,
            marks: Vec::new(),
            messages: std::iter::once(note).chain(replies).collect(),
        })
    }

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
        for (block, thread) in &threads {
            let lines = lines_of(thread, width, &mut said);
            match rows.last_mut() {
                Some((last, stack)) if last == block => stack.extend(lines),
                _ => rows.push((*block, lines)),
            }
        }
        let under = rows.iter().map(|(block, lines)| (*block, lines.len())).collect();
        Self { width, threads, rows, said, under, unfolded }
    }

    /// Whether note `id`'s replies are in view in its open box, so its bubble need not
    /// repeat them.
    pub(super) fn shows_replies(&self, id: &str) -> bool {
        self.threads.iter().any(|(_, thread)| thread.id == id && !thread.is_folded())
    }
}

impl App {
    /// Match the flow to the open document's notes and width, and keep its rows free under
    /// their blocks. When nothing changed this compares the threads and wraps nothing.
    pub(super) fn sync_threads(&mut self) {
        let threads = self.journal_threads().unwrap_or_else(|| {
            let open = &self.open;
            let placed = open.store.placed();
            placed
                .iter()
                .filter_map(|p| Some((open.doc.block_containing(p.range.start)?, Thread::of(p.annotation)?)))
                .collect()
        });
        let open = &mut self.open;
        let mut threads: Vec<(usize, Thread)> = threads;
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
            // words keeps its place in the words: its toolbar stays where it was.
            self.selection = None;
            if let Some(pending) = self.pending.as_mut()
                && let Some(range) = pending.range()
            {
                let block = open.doc.block_containing(range.start);
                let row = block.and_then(|block| open.layout.first_row_in_range(block, range));
                pending.at.0 = row.unwrap_or(pending.at.0);
            }
            self.scroll_by(0);
        }
    }

    /// The flow's rows that are in view, on the rows the layout keeps free for them, with the
    /// selection and the cursor over their words.
    pub(super) fn draw_threads(&self, buf: &mut Buffer, doc: Rect) {
        let view = self.scroll..self.scroll + usize::from(doc.height);
        for (block, rows) in &self.open.flow.rows {
            let Some(rendered) = self.open.layout.blocks.get(*block) else { continue };
            let top = rendered.first_row + rendered.rows.len();
            for row in view.start.max(top)..view.end.min(top + rows.len()) {
                let Some(drawn) = rows.get(row - top) else { continue };
                let y = doc.y + (row - self.scroll) as u16;
                buf.set_line(doc.x, y, &drawn.line, doc.width);
                self.mark_words(buf, doc, (row, y), &drawn.words);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use plannotator_tui_schema::Kind;
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

    use super::super::review_test_support::{answer_on_disk, drag_over, draw, file_app, reopen, reply_json};
    use crate::app::App;

    const DOC: &str = "# Rollout\n\nThe on-call engineer turns the new checkout off.\n\nWe ship on Friday.\n";

    /// A file review of `DOC` with a note on each paragraph; the first one answered on disk.
    fn answered_app(tag: &str, state: &str) -> (std::path::PathBuf, App) {
        let (root, mut app, _) = file_app(tag);
        std::fs::write(root.join("docs/a.md"), DOC).expect("document");
        reopen(&mut app);
        app.add_quote_annotation("turns the new checkout off", Kind::Comment, "Who watches at night?".into())
            .expect("first note");
        app.add_quote_annotation("Friday", Kind::Comment, "Not on a Friday.".into()).expect("second note");
        answer_on_disk(&mut app, 0, serde_json::json!([reply_json("Nobody today.", "claude")]), state);
        (root, app)
    }

    fn press(app: &mut App, codes: &[KeyCode]) {
        for code in codes {
            app.handle_event(&Event::Key(KeyEvent::from(*code))).expect("key");
        }
    }

    fn row_of(rows: &[&str], needle: &str) -> usize {
        rows.iter().position(|row| row.contains(needle)).expect(needle)
    }

    #[test]
    fn an_answered_note_is_a_box_right_under_the_block_its_passage_is_in() {
        let (root, mut app) = answered_app("threads-box", "open");
        // Under 80 columns the rail is gone: what is left is the document's own column.
        let screen = draw(&mut app, 70, 20);
        let rows: Vec<&str> = screen.lines().collect();
        let passage = row_of(&rows, "The on-call engineer");
        let head = row_of(&rows, "\u{256d}");
        assert_eq!(head, passage + 1, "{screen}");
        assert!(rows[head].contains("\"turns the new checkout off\""), "{screen}");
        assert!(row_of(&rows, "\u{2502} You") < row_of(&rows, "Who watches at night?"), "{screen}");
        assert!(row_of(&rows, "\u{2502} claude") < row_of(&rows, "Nobody today."), "{screen}");
        assert_eq!(row_of(&rows, "We ship on Friday."), row_of(&rows, "\u{2570}") + 2, "{screen}");
        assert!(!screen.contains("Not on a Friday."), "a note no one answered has no box: {screen}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn the_cursor_steps_over_a_box_to_the_next_text() {
        let (root, mut app) = answered_app("threads-cursor", "open");
        draw(&mut app, 70, 20);
        app.select_block(1);
        press(&mut app, &[KeyCode::Char('i'), KeyCode::Char('j')]);
        let next = app.open.layout.blocks.get(2).map(|b| b.first_row);
        assert_eq!(Some(app.cursor.0), next, "j lands on the next paragraph, not in the box");
        assert_eq!(app.selected, 2);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn the_cursor_never_stops_in_a_box_under_the_last_block() {
        let (root, mut app) = answered_app("threads-cursor-end", "open");
        answer_on_disk(&mut app, 1, serde_json::json!([reply_json("Friday is fine.", "claude")]), "open");
        draw(&mut app, 70, 30);
        app.select_block(2);
        let last = app.cursor.0;
        press(&mut app, &[KeyCode::Char('i'), KeyCode::Char('j'), KeyCode::Char('j')]);
        assert_eq!(app.cursor.0, last, "nothing but the box below: the cursor stays on the text");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn a_resolved_thread_folds_to_one_line() {
        let (root, mut app) = answered_app("threads-folded", "resolved");
        let screen = draw(&mut app, 70, 20);
        let rows: Vec<&str> = screen.lines().collect();
        let folded = row_of(&rows, "\u{2713}");
        assert_eq!(folded, row_of(&rows, "The on-call engineer") + 1, "{screen}");
        assert!(rows[folded].contains("resolved \u{b7} 2 messages"), "{screen}");
        assert!(!screen.contains("Nobody today."), "{screen}");
        assert_eq!(row_of(&rows, "We ship on Friday."), folded + 2, "{screen}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    /// `toggle-thread` answers only while the review loop runs (`app/journal`).
    #[test]
    fn without_the_review_loop_z_leaves_a_folded_thread_folded() {
        let (root, mut app) = answered_app("threads-unfold", "resolved");
        draw(&mut app, 70, 20);
        app.select_block(1);
        press(&mut app, &[KeyCode::Char('z')]);
        assert!(!draw(&mut app, 70, 20).contains("Nobody today."), "still folded");
        assert_eq!(app.status, None, "and nothing said");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    /// The words in a box take a selection only while the review loop runs (`threads::words`).
    #[test]
    fn without_the_review_loop_a_boxs_words_take_no_selection() {
        let (root, mut app) = answered_app("threads-select", "open");
        let screen = draw(&mut app, 70, 20);
        drag_over(&mut app, &screen, "Nobody today.", 0);
        assert!(app.pending.is_none() && app.selection.is_none(), "nothing selected");
        assert_eq!(app.status, None, "and nothing said");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn an_open_boxs_replies_are_counted_in_its_bubble_not_repeated() {
        let (root, mut app) = answered_app("threads-rail", "open");
        let screen = draw(&mut app, 110, 20);
        assert_eq!(screen.matches("Nobody today.").count(), 1, "{screen}");
        assert!(screen.contains("\u{21b3} 1 reply under the passage"), "{screen}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
