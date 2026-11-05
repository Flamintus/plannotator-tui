//! Threads in the document's flow. A note someone answered, or resolved, is drawn as a box
//! right under the block its passage starts in: the passage at its head, then the note and
//! each reply with who wrote it. A thread opened on one of those messages is a box inside
//! the box, under that message, at the same indent at every depth. A resolved box folds to
//! one line once nothing under it, at any depth, is open; until then it stays open, marked
//! resolved. A note no one has answered stays a bubble in the rail only.
//!
//! The layout keeps the boxes' rows free under their blocks (`DocLayout::reserve_under`),
//! so the document's own rows never move inside a block. The rows are rebuilt only when a
//! thread or the width changes.
//!
//! A document whose comments the review's journal keeps draws them from the review's view
//! (`journal`), sub-threads, sends and what is new included; `toggle-thread` draws a folded
//! thread open, or folds it again (`fold`).

mod fold;
mod journal;

use std::collections::HashSet;

use plannotator_tui_schema::{Annotation, Kind, State};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::draw::{accent, short_id};
use super::{App, bubble, glyph, label};
use crate::wrap::wrap_line;

/// Narrowest box worth drawing; a thread nested past it shows its folded line instead.
const MIN_BOX_WIDTH: usize = 16;
/// Columns of the passage a folded line keeps before it gives up the note's id.
const MIN_QUOTE_WIDTH: usize = 16;

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
    author: String,
    /// What the line naming the author adds: a draft, what became of a send, new.
    tag: Option<String>,
    body: String,
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
        let note =
            Message { author: author.to_owned(), tag: None, body: body.to_owned(), threads: Vec::new() };
        let replies = annotation.replies.iter().map(|reply| Message {
            author: bubble::author(reply).to_owned(),
            tag: None,
            body: reply.body.clone(),
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
    rows: Vec<(usize, Vec<Line<'static>>)>,
    /// How many rows that is per block: what the layout keeps free.
    under: Vec<(usize, usize)>,
    /// Folded threads drawn open by hand, by id.
    unfolded: HashSet<String>,
}

impl Flow {
    fn new(threads: Vec<(usize, Thread)>, width: usize, unfolded: HashSet<String>) -> Self {
        let mut rows: Vec<(usize, Vec<Line<'static>>)> = Vec::new();
        for (block, thread) in &threads {
            let lines = lines_of(thread, width);
            match rows.last_mut() {
                Some((last, stack)) if last == block => stack.extend(lines),
                _ => rows.push((*block, lines)),
            }
        }
        let under = rows.iter().map(|(block, lines)| (*block, lines.len())).collect();
        Self { width, threads, rows, under, unfolded }
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
            // range, which has not moved: it only follows its first row.
            self.selection = None;
            if let Some(pending) = self.pending.as_mut() {
                let block = open.doc.block_containing(pending.range.start);
                let row = block.and_then(|block| open.layout.first_row_in_range(block, &pending.range));
                pending.at.0 = row.unwrap_or(pending.at.0);
            }
            self.scroll_by(0);
        }
    }

    /// The flow's rows that are in view, on the rows the layout keeps free for them.
    pub(super) fn draw_threads(&self, buf: &mut Buffer, doc: Rect) {
        let view = self.scroll..self.scroll + usize::from(doc.height);
        for (block, lines) in &self.open.flow.rows {
            let Some(rendered) = self.open.layout.blocks.get(*block) else { continue };
            let top = rendered.first_row + rendered.rows.len();
            for row in view.start.max(top)..view.end.min(top + lines.len()) {
                let Some(line) = lines.get(row - top) else { continue };
                buf.set_line(doc.x, doc.y + (row - self.scroll) as u16, line, doc.width);
            }
        }
    }
}

/// A thread's rows, `width` columns wide: its folded line once settled, else its box.
fn lines_of(thread: &Thread, width: usize) -> Vec<Line<'static>> {
    if thread.is_folded() || width < MIN_BOX_WIDTH {
        return vec![folded(thread, width)];
    }
    let border =
        if thread.resolved { Style::new().fg(Color::DarkGray) } else { Style::new().fg(accent(thread.kind)) };
    let text = if thread.resolved { Style::new().add_modifier(Modifier::DIM) } else { Style::new() };
    let inner = width - 4;
    let mut lines = vec![head(thread, width, border)];
    for (index, message) in thread.messages.iter().enumerate() {
        if index > 0 {
            lines.push(edge('├', '┤', width, border));
        }
        let named = match &message.tag {
            Some(tag) => format!("{} \u{b7} {tag}", one_line(&message.author)),
            None => one_line(&message.author),
        };
        let author = Line::styled(fit(&named, inner), text.add_modifier(Modifier::BOLD));
        lines.push(framed(author, inner, border));
        for paragraph in printable(&message.body).lines() {
            let rows = wrap_line(&Line::styled(paragraph.to_owned(), text), &[], inner);
            if rows.is_empty() {
                lines.push(framed(Line::default(), inner, border));
            }
            lines.extend(rows.into_iter().map(|row| framed(row.line, inner, border)));
        }
        for sub in &message.threads {
            lines.extend(lines_of(sub, inner).into_iter().map(|line| framed(line, inner, border)));
        }
    }
    lines.push(edge('╰', '╯', width, border));
    lines
}

/// `╭ 💬 "passage" ──── resolved · 3e05f ╮`: the passage cut to fit, the state and the
/// tail of the note's id, which its bubble's title shows too.
/// In a narrow box the state goes first, then the id, before the passage gets too short.
fn head(thread: &Thread, width: usize, border: Style) -> Line<'static> {
    let lead = format!("\u{256d} {} ", glyph(thread.kind));
    let id = short_id(&thread.id);
    // The state and the marks, then fewer of them, then the id alone, until the passage fits.
    let mut marks: Vec<&str> = thread
        .resolved
        .then_some("resolved")
        .into_iter()
        .chain(thread.marks.iter().map(String::as_str))
        .collect();
    let mut rights = Vec::new();
    while !marks.is_empty() {
        rights.push(format!(" {} \u{b7} {id} ", marks.join(" \u{b7} ")));
        marks.pop();
    }
    rights.push(format!(" {id} "));
    let passage = one_line(&thread.quote);
    let wanted = passage.width().min(MIN_QUOTE_WIDTH);
    let right = rights
        .into_iter()
        .find(|right| width.saturating_sub(lead.width() + right.width() + 4) >= wanted)
        .unwrap_or_else(|| " ".to_owned());
    let room = width.saturating_sub(lead.width() + right.width() + 4);
    let quote = format!("\"{}\"", fit(&passage, room));
    let fill = width.saturating_sub(lead.width() + quote.width() + right.width() + 2);
    Line::from(vec![
        Span::styled(lead, border),
        Span::styled(quote, border.add_modifier(Modifier::ITALIC)),
        Span::styled(format!(" {}", "\u{2500}".repeat(fill)), border),
        Span::styled(right, Style::new().fg(Color::DarkGray)),
        Span::styled("\u{256e}", border),
    ])
}

/// A folded thread in one line: `╶ ✓ 💬 "passage" · resolved · 3 messages · 1 sub-thread ─╴`.
fn folded(thread: &Thread, width: usize) -> Line<'static> {
    let (messages, threads) = thread.size();
    let mark = if thread.resolved { "\u{2713}" } else { "\u{25b8}" };
    let state = if thread.resolved { "resolved" } else { "open" };
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let counts = match threads {
        0 => format!("{messages} message{}", plural(messages)),
        _ => format!("{messages} message{} \u{b7} {threads} sub-thread{}", plural(messages), plural(threads)),
    };
    let lead = format!("\u{2576} {mark} {} ", glyph(thread.kind));
    let passage = one_line(&thread.quote);
    // The passage tells folded threads apart better than the id: the id goes first.
    let mut tail = format!(" \u{b7} {state} \u{b7} {counts} \u{b7} {} ", short_id(&thread.id));
    if width.saturating_sub(lead.width() + tail.width() + 3) < passage.width().min(MIN_QUOTE_WIDTH) {
        tail = format!(" \u{b7} {state} \u{b7} {counts} ");
    }
    let room = width.saturating_sub(lead.width() + tail.width() + 3);
    let quote = format!("\"{}\"", fit(&passage, room));
    let fill = width.saturating_sub(lead.width() + quote.width() + tail.width() + 1);
    let style = Style::new().fg(Color::DarkGray);
    let line = format!("{lead}{quote}{tail}{}\u{2574}", "\u{2500}".repeat(fill));
    Line::styled(fit(&line, width), style)
}

/// `left` and `right` with a rule between them, `width` columns in all.
fn edge(left: char, right: char, width: usize, border: Style) -> Line<'static> {
    Line::styled(format!("{left}{}{right}", "\u{2500}".repeat(width.saturating_sub(2))), border)
}

/// `line` between the box's sides, padded to `inner` columns.
fn framed(line: Line<'static>, inner: usize, border: Style) -> Line<'static> {
    let pad = inner.saturating_sub(line.width());
    let mut spans = vec![Span::styled("\u{2502} ", border)];
    spans.extend(line.spans.into_iter().map(|span| {
        let style = line.style.patch(span.style);
        span.style(style)
    }));
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(" \u{2502}", border));
    Line::from(spans)
}

/// `text` as the terminal draws it: a tab as four spaces, other control characters (an
/// escape sequence's ESC, a carriage return) left out, so its width is what is drawn.
fn printable(text: &str) -> String {
    text.replace('\t', "    ").chars().filter(|c| *c == '\n' || !c.is_control()).collect()
}

/// `printable`, on one line with single spaces.
fn one_line(text: &str) -> String {
    printable(text).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` cut to `max` columns, ending in `…` when cut.
fn fit(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if out.width() + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) + 1 > max {
            break;
        }
        out.push(ch);
    }
    out.push('\u{2026}');
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use plannotator_tui_schema::Kind;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};
    use ratatui::layout::Rect;

    use super::super::review_test_support::{answer_on_disk, draw, file_app, reopen, reply_json};
    use super::{Message, Thread, lines_of};
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

    fn message(author: &str, body: &str, threads: Vec<Thread>) -> Message {
        Message { author: author.into(), tag: None, body: body.into(), threads }
    }

    fn thread(id: &str, resolved: bool, messages: Vec<Message>) -> Thread {
        let quote = format!("passage {id}");
        Thread {
            id: id.into(),
            kind: Kind::Comment,
            quote,
            resolved,
            folded: None,
            marks: Vec::new(),
            messages,
        }
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

    #[test]
    fn a_folded_thread_is_drawn_open_by_hand_and_folded_again() {
        let (root, mut app) = answered_app("threads-unfold", "resolved");
        draw(&mut app, 70, 20);
        app.select_block(1);
        press(&mut app, &[KeyCode::Char('z')]);
        let screen = draw(&mut app, 70, 20);
        assert!(screen.contains("Nobody today."), "{screen}");
        let rows: Vec<&str> = screen.lines().collect();
        assert!(rows[row_of(&rows, "\u{256d}")].contains("resolved"), "still resolved: {screen}");
        press(&mut app, &[KeyCode::Char('z')]);
        assert!(!draw(&mut app, 70, 20).contains("Nobody today."), "folded again");
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

    #[test]
    fn a_boxs_right_edge_is_its_last_column_whatever_the_width_or_the_text() {
        let pasted = "let x =\tcolumns();\u{1b}[31m red\r";
        let sub = thread("b", false, vec![message("You", pasted, Vec::new())]);
        let top = thread("a", true, vec![message("claude\t", pasted, vec![sub])]);
        for width in 16..=60u16 {
            let lines = lines_of(&top, usize::from(width));
            let mut buf = Buffer::empty(Rect::new(0, 0, width, lines.len() as u16));
            for (y, line) in lines.iter().enumerate() {
                buf.set_line(0, y as u16, line, width);
                let edge = buf.cell((width - 1, y as u16)).map(ratatui::buffer::Cell::symbol);
                assert!(
                    matches!(edge, Some("\u{256e}" | "\u{2502}" | "\u{2524}" | "\u{256f}")),
                    "{width} columns, row {y}: {edge:?} in {line}"
                );
            }
        }
    }

    #[test]
    fn a_resolved_thread_stays_open_while_anything_under_it_at_any_depth_is_open() {
        let deepest = thread("c", false, vec![message("You", "Still open.", Vec::new())]);
        let middle = thread("b", true, vec![message("claude", "Settled here.", vec![deepest])]);
        let mut top = thread("a", true, vec![message("You", "The note.", vec![middle])]);
        let lines: Vec<String> = lines_of(&top, 60).iter().map(ToString::to_string).collect();
        assert!(lines.len() > 1, "{lines:#?}");
        assert!(lines[0].contains("resolved"), "marked resolved while open: {lines:#?}");
        let nested = lines.iter().filter(|l| l.starts_with("\u{2502} \u{2502} \u{256d}")).count();
        assert_eq!(nested, 1, "the deepest thread is a box in a box in the box: {lines:#?}");
        assert!(lines.iter().any(|l| l.contains("Still open.")), "{lines:#?}");

        top.messages[0].threads[0].messages[0].threads[0].resolved = true;
        let lines = lines_of(&top, 60);
        assert_eq!(lines.len(), 1, "everything resolved: one line");
        assert!(lines[0].to_string().contains("3 messages \u{b7} 2 sub-threads"), "{lines:#?}");
    }
}
