//! A thread's rows as drawn, at every depth and of every kind: its box, its head (`head`), each
//! message under who wrote it, the threads on a message's words as boxes in the box, each under
//! the paragraph its passage starts in, a blank row above and below each (`spaced`), and the
//! field the person types in, a box in the box too, at its place (`field`): a reply's at the
//! bottom, a sub-thread's among the boxes under the paragraph its passage starts in, an edit's
//! in place of the note's words; or, folded, its folded line. Each row keeps, per column, the
//! message's words it shows (`words::Spot`), and a thread's first row says when it is its fold
//! line. Every box's border is grey (`head::BORDER`); each row keeps which of its columns are
//! whose border, so the box the person is in is drawn lit over them, and moving from one box to
//! another rebuilds no row.

use std::ops::Range;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::field::{Field, Place};
use super::head::{BORDER, folded, head, mark_style};
use super::words::{Said, Spot};
use super::{Message, Thread};
use crate::doc::{block_holding, plain_blocks};
use crate::theme::palette;
use crate::wrap::wrap_line;

/// Narrowest box worth drawing; a thread nested past it shows its folded line instead.
const MIN_BOX_WIDTH: usize = 16;
/// One row of the flow: what is drawn, the innermost thread it belongs to, and, per column,
/// the message's words it shows.
#[derive(Debug)]
pub(super) struct BoxRow {
    pub(super) line: Line<'static>,
    /// The id of the thread whose box (or folded line) the row is in, the innermost one.
    pub(super) thread: Option<String>,
    /// Per column, the words shown; a row that shows none has a column per side of the boxes
    /// it is in.
    pub(super) words: Vec<Option<Spot>>,
    /// The row is its thread's fold line, where `down` and `up` stop: whether the thread is
    /// folded (its folded line, which `right` or a click draws open) or open (the head of its
    /// box, which `left` or a click folds).
    pub(super) fold: Option<bool>,
    /// The row is one of the words of the field the person types in, drawn over it (`field`).
    pub(super) field: bool,
    /// The columns that are a thread's border, with the thread: the sides of each box the row is
    /// in, a head but its marks, a rule, a bottom edge, a folded line's mark and rule.
    pub(super) border: Vec<(String, Range<usize>)>,
}

impl BoxRow {
    /// A row that shows no message's words: an author, a blank row, a field's edge.
    fn plain(line: Line<'static>) -> Self {
        Self { line, thread: None, words: Vec::new(), fold: None, field: false, border: Vec::new() }
    }

    /// A row of thread `id`'s own border, its `columns`: its head, a rule, its bottom edge, its
    /// folded line.
    fn own(line: Line<'static>, id: &str, columns: impl IntoIterator<Item = Range<usize>>) -> Self {
        let border = columns.into_iter().map(|columns| (id.to_owned(), columns)).collect();
        Self { border, ..Self::plain(line) }
    }

    /// A row that shows nothing: a blank line of a message's words.
    fn is_blank(&self) -> bool {
        self.line.width() == 0
    }

    /// The row between the sides of a box `inner` columns wide, thread `of`'s (none for the
    /// field the person types in).
    fn in_box(self, inner: usize, border: Style, of: Option<&str>) -> Self {
        let words = [None, None].into_iter().chain(self.words).collect();
        let within = self.border.into_iter().map(|(id, columns)| (id, columns.start + 2..columns.end + 2));
        let sides =
            of.into_iter().flat_map(|id| [(id.to_owned(), 0..2), (id.to_owned(), inner + 2..inner + 4)]);
        Self { line: framed(self.line, inner, border), words, border: sides.chain(within).collect(), ..self }
    }
}

/// Columns inside a box `width` columns wide, between its sides: what its messages' words, a
/// box in it and the field the person types in take.
pub(super) fn inside(width: usize) -> usize {
    width.saturating_sub(4)
}

/// A box under a line of words: a thread's, or the field a comment on them is typed in.
#[derive(Debug, Clone, Copy)]
pub(super) enum Under<'t> {
    Thread(&'t Thread),
    Field(&'t Field),
}

/// The boxes `under` one line of the words they are on (a block of the document, a line of a
/// message's words in a box), `width` columns wide: a blank row above each and one below the
/// last, so every box has one above it and one below it, at every depth and of every kind, the
/// field's too. `blank_follows`: the words go on under them with a blank row of their own (the
/// gap under a block, a blank line of a message), which is the one below.
pub(super) fn spaced<'t>(
    under: impl IntoIterator<Item = Under<'t>>,
    width: usize,
    said: &mut Vec<Said>,
    blank_follows: bool,
) -> Vec<BoxRow> {
    let mut rows = Vec::new();
    for boxed in under {
        rows.push(BoxRow::plain(Line::default()));
        rows.extend(match boxed {
            Under::Thread(thread) => lines_of(thread, width, said),
            Under::Field(field) => field_of(field, width),
        });
    }
    if !rows.is_empty() && !blank_follows {
        rows.push(BoxRow::plain(Line::default()));
    }
    rows
}

/// A thread's rows, `width` columns wide: its folded line once folded, else its box. The
/// messages whose words the box shows join `said`, and its rows name them by their place there;
/// each row names the innermost thread it is in. The first is its fold line: its folded line,
/// or the head of its box; a box too narrow to draw shows its folded line, which is none.
fn lines_of(thread: &Thread, width: usize, said: &mut Vec<Said>) -> Vec<BoxRow> {
    let boxed = !thread.is_folded() && width >= MIN_BOX_WIDTH;
    let mut rows = if boxed {
        box_of(thread, width, said)
    } else {
        let (line, border) = folded(thread, width);
        vec![BoxRow::own(line, &thread.id, border)]
    };
    if let Some(first) = rows.first_mut() {
        first.fold = if thread.is_folded() { Some(true) } else { boxed.then_some(false) };
    }
    for row in &mut rows {
        row.thread.get_or_insert_with(|| thread.id.clone());
    }
    rows
}

/// A thread's box: its head, then each message under who wrote it, the threads on its words
/// (`spaced`) under the paragraph their passage starts in, as a thread is under the block its
/// passage starts in; one whose passage is not found there, after the message's words. The
/// field the person types in, a box in the box with a blank row above and below it: an edit's in
/// place of the note's words, before the threads on them; a sub-thread's among those under the
/// paragraph its passage starts in, after those starting where it does, as its box will be,
/// when that box is wide enough to draw; last, a reply's.
fn box_of(thread: &Thread, width: usize, said: &mut Vec<Said>) -> Vec<BoxRow> {
    let id = Some(thread.id.as_str());
    let text = if thread.resolved { Style::new().add_modifier(Modifier::DIM) } else { Style::new() };
    let inner = inside(width);
    let field = thread.field.as_ref();
    let (head, columns) = head(thread, width);
    let mut rows = vec![BoxRow::own(head, &thread.id, columns)];
    for (index, message) in thread.messages.iter().enumerate() {
        if index > 0 {
            rows.push(separator(width, &thread.id));
        }
        // Who wrote it, then the state it is still in, in that state's colour (`mark_style`).
        let name = one_line(&message.author);
        let author = match &message.tag {
            Some(tag) => Line::from(vec![
                Span::styled(
                    fit(&name, inner.saturating_sub(tag.width() + 3)),
                    text.add_modifier(Modifier::BOLD),
                ),
                Span::styled(" \u{b7} ", text),
                Span::styled(tag.clone(), mark_style(tag)),
            ]),
            None => Line::styled(fit(&name, inner), text.add_modifier(Modifier::BOLD)),
        };
        rows.push(BoxRow::plain(author).in_box(inner, BORDER, id));
        let edited = field.filter(|field| index == 0 && field.place == Place::Words);
        let lines = if edited.is_some() { Vec::new() } else { words_of(message, inner, text, said) };
        let passage = field.filter(|_| inner >= MIN_BOX_WIDTH).and_then(|field| match field.place {
            Place::Passage { message, start } if message == index => Some((field, start)),
            Place::Passage { .. } | Place::Bottom | Place::Words => None,
        });
        let mut under = under_lines(message, &lines, passage);
        if let (Some(field), Some(after)) = (edited, under.last_mut()) {
            after.insert(0, Under::Field(field));
        }
        let mut lines = lines.into_iter().map(Some).chain([None]).zip(under).peekable();
        while let Some((line, boxes)) = lines.next() {
            rows.extend(line.map(|line| line.in_box(inner, BORDER, id)));
            let blank = lines.peek().is_some_and(|(next, _)| next.as_ref().is_some_and(BoxRow::is_blank));
            let boxes = spaced(boxes, inner, said, blank);
            rows.extend(boxes.into_iter().map(|row| row.in_box(inner, BORDER, id)));
        }
    }
    if let Some(field) = field.filter(|field| field.place == Place::Bottom) {
        let blank = || BoxRow::plain(Line::default()).in_box(inner, BORDER, id);
        // The blank row under the boxes the last message ends with is the one above the field.
        if !rows.last().is_some_and(|row| row.line == blank().line) {
            rows.push(blank());
        }
        rows.extend(field_of(field, inner).into_iter().map(|row| row.in_box(inner, BORDER, id)));
        rows.push(blank());
    }
    rows.push(rule('╰', '╯', width, &thread.id));
    rows
}

/// The field the person types in, a box `width` columns wide drawn as the compose box is: its
/// title at its head, then a row of its sides for each row of the words, which are drawn over
/// them.
fn field_of(field: &Field, width: usize) -> Vec<BoxRow> {
    let border = Style::new().fg(palette().comment);
    let title = fit(&field.title, width.saturating_sub(2));
    let rule = "\u{2500}".repeat(width.saturating_sub(2 + title.width()));
    let head = Line::from(vec![
        Span::styled("\u{256d}", border),
        Span::styled(title, Style::new().add_modifier(Modifier::DIM)),
        Span::styled(format!("{rule}\u{256e}"), border),
    ]);
    let words = (0..field.rows).map(|_| BoxRow { field: true, ..BoxRow::plain(Line::default()) });
    std::iter::once(BoxRow::plain(head))
        .chain(words.map(|row| row.in_box(inside(width), border, None)))
        .chain([BoxRow::plain(edge('╰', '╯', width, border))])
        .collect()
}

/// A message's words wrapped to `inner` columns, each row knowing the byte of the words each
/// column shows; the message joins `said`, with the passages of the threads on its words.
fn words_of(message: &Message, inner: usize, text: Style, said: &mut Vec<Said>) -> Vec<BoxRow> {
    let spoken = said.len();
    let passages = message.threads.iter().filter_map(|sub| Some((sub.range.clone()?, sub.kind))).collect();
    said.push(Said { id: message.id.clone(), words: message.body.clone(), draft: message.draft, passages });
    let mut lines = Vec::new();
    for paragraph in printable_lines(&message.body) {
        let (words, bytes): (String, Vec<Option<usize>>) =
            paragraph.into_iter().map(|(ch, byte)| (ch, Some(byte))).unzip();
        for row in wrap_line(&Line::styled(words, text), &bytes, inner) {
            let spots = row.cells.iter().map(|cell| cell.map(|byte| Spot { message: spoken, byte }));
            lines.push(BoxRow { line: row.line, words: spots.collect(), ..BoxRow::plain(Line::default()) });
        }
    }
    lines
}

/// The threads on `message`'s words under each of its `lines`, and the field a comment on a
/// passage of them is typed in (`field`, with the byte the passage starts at): under the last
/// line of the paragraph their passage starts in, as a thread is under the document's block
/// (`doc::block_holding`), in their passages' order, the field after the threads starting where
/// it does; last, those whose passage is not found.
fn under_lines<'t>(
    message: &'t Message,
    lines: &[BoxRow],
    field: Option<(&'t Field, usize)>,
) -> Vec<Vec<Under<'t>>> {
    let paragraphs = plain_blocks(&message.body);
    let paragraph_of = |byte: usize| block_holding(&paragraphs, byte);
    let of_line: Vec<Option<usize>> = lines
        .iter()
        .map(|line| line.words.iter().flatten().map(|spot| spot.byte).min().and_then(paragraph_of))
        .collect();
    let mut under = vec![Vec::new(); lines.len() + 1];
    let threads =
        message.threads.iter().map(|sub| (sub.range.as_ref().map(|range| range.start), Under::Thread(sub)));
    let field = field.map(|(field, start)| (Some(start), Under::Field(field)));
    let mut boxes: Vec<(Option<usize>, Under<'t>)> = threads.chain(field).collect();
    boxes.sort_by_key(|(start, _)| *start);
    for (start, boxed) in boxes {
        let paragraph = start.and_then(paragraph_of);
        let line = paragraph.and_then(|paragraph| of_line.iter().rposition(|of| *of == Some(paragraph)));
        if let Some(at) = under.get_mut(line.unwrap_or(lines.len())) {
            at.push(boxed);
        }
    }
    under
}

/// Thread `id`'s rule from `left` to `right`, `width` columns, all of them its border.
fn rule(left: char, right: char, width: usize, id: &str) -> BoxRow {
    BoxRow::own(edge(left, right, width, BORDER), id, std::iter::once(0..width))
}

/// The separator between two messages of a box, as discreet as can be: its junctions are the
/// box's border, lit with it on the current box; the dotted rule between them stays dim grey.
fn separator(width: usize, id: &str) -> BoxRow {
    let line = Line::from(vec![
        Span::styled("\u{251c}", BORDER),
        Span::styled("\u{2508}".repeat(width.saturating_sub(2)), BORDER.add_modifier(Modifier::DIM)),
        Span::styled("\u{2524}", BORDER),
    ]);
    BoxRow::own(line, id, [0..1, width.saturating_sub(1)..width])
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

/// `printable`'s lines, each character with the byte of `text` it shows (a tab's four spaces
/// all show the tab), as `str::lines` splits them.
fn printable_lines(text: &str) -> Vec<Vec<(char, usize)>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for piece in text.split_inclusive('\n') {
        let mut line = Vec::new();
        for (at, ch) in piece.char_indices() {
            match ch {
                '\t' => line.extend(std::iter::repeat_n((' ', start + at), 4)),
                ch if ch.is_control() => {}
                ch => line.push((ch, start + at)),
            }
        }
        lines.push(line);
        start += piece.len();
    }
    lines
}

/// `printable`, on one line with single spaces.
pub(super) fn one_line(text: &str) -> String {
    printable(text).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` cut to `max` columns, ending in `…` when cut.
pub(super) fn fit(text: &str, max: usize) -> String {
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
    use ratatui::layout::Rect;

    use super::super::head::{Mark, RESOLVED};
    use super::super::{Message, Thread};
    use super::{Line, lines_of};
    use crate::theme::palette;

    /// `lines_of`'s lines alone.
    fn drawn(thread: &Thread, width: usize) -> Vec<Line<'static>> {
        lines_of(thread, width, &mut Vec::new()).into_iter().map(|row| row.line).collect()
    }

    fn message(author: &str, body: &str, threads: Vec<Thread>) -> Message {
        Message {
            id: format!("m-{body}"),
            author: author.into(),
            tag: None,
            body: body.into(),
            draft: false,
            threads,
        }
    }

    fn thread(id: &str, resolved: bool, messages: Vec<Message>) -> Thread {
        let quote = format!("passage {id}");
        Thread {
            id: id.into(),
            kind: Kind::Comment,
            quote,
            range: None,
            resolved,
            folded: None,
            by_hand: None,
            marks: resolved.then_some(Mark::State(RESOLVED)).into_iter().collect(),
            images: 0,
            messages,
            field: None,
        }
    }

    #[test]
    fn a_boxs_right_edge_is_its_last_column_whatever_the_width_or_the_text() {
        let pasted = "let x =\tcolumns();\u{1b}[31m red\r";
        let sub = thread("b", false, vec![message("You", pasted, Vec::new())]);
        let top = thread("a", true, vec![message("claude\t", pasted, vec![sub])]);
        for width in 16..=60u16 {
            let lines = drawn(&top, usize::from(width));
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
        let lines: Vec<String> = drawn(&top, 60).iter().map(ToString::to_string).collect();
        assert!(lines.len() > 1, "{lines:#?}");
        assert!(lines[0].contains("Resolved"), "marked resolved while open: {lines:#?}");
        let nested = lines.iter().filter(|l| l.starts_with("\u{2502} \u{2502} \u{256d}")).count();
        assert_eq!(nested, 1, "the deepest thread is a box in a box in the box: {lines:#?}");
        assert!(lines.iter().any(|l| l.contains("Still open.")), "{lines:#?}");

        top.messages[0].threads[0].messages[0].threads[0].resolved = true;
        let lines = drawn(&top, 60);
        assert_eq!(lines.len(), 1, "everything resolved: one line");
        assert!(lines[0].to_string().contains("1 message \u{b7} 2 sub-threads"), "{lines:#?}");
    }

    #[test]
    fn the_resolved_mark_takes_the_resolved_colour_open_or_folded() {
        let open_under = thread("b", false, vec![message("You", "Still open.", Vec::new())]);
        let held_open = thread("a", true, vec![message("You", "The note.", vec![open_under])]);
        let folded = thread("c", true, vec![message("You", "Done.", Vec::new())]);
        for top in [&held_open, &folded] {
            let first = drawn(top, 60).into_iter().next().expect("a head or a folded line");
            let mark = first.spans.iter().find(|span| span.content == RESOLVED);
            assert_eq!(mark.and_then(|span| span.style.fg), Some(palette().resolved), "{first}");
        }
    }
}
