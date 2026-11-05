//! A thread's rows as drawn: its box, the passage and the marks at its head, each message
//! under who wrote it, the threads on a message's words as boxes in the box, each under the
//! line its passage starts on; or, settled, one folded line. Each row keeps, per column, the
//! message's words it shows (`words::Spot`).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::super::draw::{accent, short_id};
use super::super::glyph;
use super::Thread;
use super::words::{Said, Spot};
use crate::theme::palette;
use crate::wrap::wrap_line;

/// Narrowest box worth drawing; a thread nested past it shows its folded line instead.
const MIN_BOX_WIDTH: usize = 16;
/// Columns of the passage a folded line keeps before it gives up the note's id.
const MIN_QUOTE_WIDTH: usize = 16;

/// The mark of a resolved thread: at its box's head, on its folded line, in its bubble's title.
pub(in crate::app) const RESOLVED: &str = "\u{2713} Resolved";

/// How the resolved mark is drawn: in the looks-good colour, so it stands apart from the grey
/// of the rest of the line.
pub(in crate::app) fn resolved_style() -> Style {
    Style::new().fg(palette().looks_good).add_modifier(Modifier::BOLD)
}

/// One row of the flow: what is drawn, the innermost thread it belongs to, and, per column,
/// the message's words it shows.
#[derive(Debug)]
pub(super) struct BoxRow {
    pub(super) line: Line<'static>,
    /// The id of the thread whose box (or folded line) the row is in, the innermost one.
    pub(super) thread: Option<String>,
    pub(super) words: Vec<Option<Spot>>,
}

impl BoxRow {
    /// A row that shows no message's words: a head, a rule, an author, a folded line.
    fn plain(line: Line<'static>) -> Self {
        Self { line, thread: None, words: Vec::new() }
    }

    /// The row between the sides of a box `inner` columns wide.
    fn in_box(self, inner: usize, border: Style) -> Self {
        let words = [None, None].into_iter().chain(self.words).collect();
        Self { line: framed(self.line, inner, border), thread: self.thread, words }
    }
}

/// A thread's rows, `width` columns wide: its folded line once settled, else its box. The
/// messages whose words the box shows join `said`, and its rows name them by their place there;
/// each row names the innermost thread it is in.
pub(super) fn lines_of(thread: &Thread, width: usize, said: &mut Vec<Said>) -> Vec<BoxRow> {
    let mut rows = if thread.is_folded() || width < MIN_BOX_WIDTH {
        vec![BoxRow::plain(folded(thread, width))]
    } else {
        box_of(thread, width, said)
    };
    for row in &mut rows {
        row.thread.get_or_insert_with(|| thread.id.clone());
    }
    rows
}

/// A thread's box, its sub-threads' rows inside.
fn box_of(thread: &Thread, width: usize, said: &mut Vec<Said>) -> Vec<BoxRow> {
    let border =
        if thread.resolved { Style::new().fg(Color::DarkGray) } else { Style::new().fg(accent(thread.kind)) };
    let text = if thread.resolved { Style::new().add_modifier(Modifier::DIM) } else { Style::new() };
    let inner = width - 4;
    let mut rows = vec![BoxRow::plain(head(thread, width, border))];
    for (index, message) in thread.messages.iter().enumerate() {
        if index > 0 {
            rows.push(BoxRow::plain(edge('├', '┤', width, border)));
        }
        let named = match &message.tag {
            Some(tag) => format!("{} \u{b7} {tag}", one_line(&message.author)),
            None => one_line(&message.author),
        };
        let author = Line::styled(fit(&named, inner), text.add_modifier(Modifier::BOLD));
        rows.push(BoxRow::plain(author).in_box(inner, border));
        let spoken = said.len();
        said.push(Said { id: message.id.clone(), words: message.body.clone(), draft: message.draft });
        let mut lines = Vec::new();
        for paragraph in printable_lines(&message.body) {
            let (words, bytes): (String, Vec<Option<usize>>) =
                paragraph.into_iter().map(|(ch, byte)| (ch, Some(byte))).unzip();
            for row in wrap_line(&Line::styled(words, text), &bytes, inner) {
                let spots = row.cells.iter().map(|cell| cell.map(|byte| Spot { message: spoken, byte }));
                lines.push(BoxRow { line: row.line, thread: None, words: spots.collect() });
            }
        }
        // Each sub-thread right under the line its passage starts on, as a thread is under the
        // block its passage starts in; one whose words are not found there, under the message.
        let firsts: Vec<Option<usize>> =
            lines.iter().map(|line| line.words.iter().flatten().map(|spot| spot.byte).min()).collect();
        let line_of = |sub: &Thread| {
            let start = sub.start?;
            firsts.iter().rposition(|first| first.is_some_and(|first| first <= start))
        };
        let mut subs: Vec<(usize, &Thread)> =
            message.threads.iter().map(|sub| (line_of(sub).unwrap_or(usize::MAX), sub)).collect();
        subs.sort_by_key(|(line, _)| *line);
        let mut subs = subs.into_iter().peekable();
        for (at, line) in lines.into_iter().enumerate() {
            rows.push(line.in_box(inner, border));
            while let Some((_, sub)) = subs.next_if(|(line, _)| *line == at) {
                rows.extend(sub_rows(sub, inner, border, said));
            }
        }
        for (_, sub) in subs {
            rows.extend(sub_rows(sub, inner, border, said));
        }
    }
    rows.push(BoxRow::plain(edge('╰', '╯', width, border)));
    rows
}

/// A sub-thread's rows inside its parent's box, `inner` columns wide: a blank row between the
/// parent's sides keeps it apart from the words over it, as a thread is from its block.
fn sub_rows(sub: &Thread, inner: usize, border: Style, said: &mut Vec<Said>) -> Vec<BoxRow> {
    std::iter::once(BoxRow::plain(Line::default()))
        .chain(lines_of(sub, inner, said))
        .map(|row| row.in_box(inner, border))
        .collect()
}

/// `╭ 💬 "passage" ──── Replied · Live · Round 2 · 3e05f ╮`: the passage cut to fit, the state,
/// the marks and the tail of the note's id, which its bubble's title shows too.
/// In a narrow box the marks go first, the last first, then the state, then the id, before the
/// passage gets too short.
fn head(thread: &Thread, width: usize, border: Style) -> Line<'static> {
    let lead = format!("\u{256d} {} ", glyph(thread.kind));
    let id = short_id(&thread.id);
    let chips: Vec<&str> = thread
        .resolved
        .then_some(RESOLVED)
        .into_iter()
        .chain(thread.marks.iter().map(String::as_str))
        .collect();
    let mut rights: Vec<Vec<&str>> = (1..=chips.len())
        .rev()
        .map(|kept| chips.iter().take(kept).copied().chain([id.as_str()]).collect())
        .collect();
    rights.push(vec![id.as_str()]);
    let passage = one_line(&thread.quote);
    let wanted = passage.width().min(MIN_QUOTE_WIDTH);
    let right = rights
        .iter()
        .find(|right| width.saturating_sub(lead.width() + chips_width(right) + 4) >= wanted)
        .map_or(&[][..], Vec::as_slice);
    let room = width.saturating_sub(lead.width() + chips_width(right) + 4);
    let quote = format!("\"{}\"", fit(&passage, room));
    let fill = width.saturating_sub(lead.width() + quote.width() + chips_width(right) + 2);
    let mut spans = vec![
        Span::styled(lead, border),
        Span::styled(quote, border.add_modifier(Modifier::ITALIC)),
        Span::styled(format!(" {}", "\u{2500}".repeat(fill)), border),
    ];
    spans.extend(chips_spans(right));
    spans.push(Span::styled("\u{256e}", border));
    Line::from(spans)
}

/// Columns `chips_spans` takes: the chips with their separators and a space each side; one
/// space for none.
fn chips_width(chips: &[&str]) -> usize {
    if chips.is_empty() { 1 } else { chips.join(" \u{b7} ").width() + 2 }
}

/// ` ✓ Resolved · Round 2 · 3e05f `: the resolved mark in its colour, the rest grey.
fn chips_spans(chips: &[&str]) -> Vec<Span<'static>> {
    let grey = Style::new().fg(Color::DarkGray);
    let mut spans = vec![Span::styled(" ", grey)];
    for (index, chip) in chips.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" \u{b7} ", grey));
        }
        let style = if *chip == RESOLVED { resolved_style() } else { grey };
        spans.push(Span::styled((*chip).to_owned(), style));
    }
    if !chips.is_empty() {
        spans.push(Span::styled(" ", grey));
    }
    spans
}

/// A folded thread in one line:
/// `╶ ✓ Resolved · 💬 "passage" · Round 2 · 3 messages · 1 sub-thread · 3e05f ─╴`.
fn folded(thread: &Thread, width: usize) -> Line<'static> {
    let (messages, threads) = thread.size();
    let state = if thread.resolved { RESOLVED } else { "\u{25b8} Open" };
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let counts = match threads {
        0 => format!("{messages} message{}", plural(messages)),
        _ => format!("{messages} message{} \u{b7} {threads} sub-thread{}", plural(messages), plural(threads)),
    };
    let lead = format!("\u{2576} {state} \u{b7} {} ", glyph(thread.kind));
    let passage = one_line(&thread.quote);
    let marks: String = thread.marks.iter().flat_map(|mark| [" \u{b7} ", mark.as_str()]).collect();
    // The passage tells folded threads apart better than the id and the marks: they go first.
    let tails = [
        format!("{marks} \u{b7} {counts} \u{b7} {} ", short_id(&thread.id)),
        format!("{marks} \u{b7} {counts} "),
        format!(" \u{b7} {counts} "),
    ];
    let wanted = passage.width().min(MIN_QUOTE_WIDTH);
    let tail = tails
        .iter()
        .find(|tail| width.saturating_sub(lead.width() + tail.width() + 3) >= wanted)
        .or_else(|| tails.last())
        .cloned()
        .unwrap_or_default();
    let room = width.saturating_sub(lead.width() + tail.width() + 3);
    let quote = format!("\"{}\"", fit(&passage, room));
    let fill = width.saturating_sub(lead.width() + quote.width() + tail.width() + 1);
    let line = fit(&format!("{lead}{quote}{tail}{}\u{2574}", "\u{2500}".repeat(fill)), width);
    let grey = Style::new().fg(Color::DarkGray);
    match line.strip_prefix("\u{2576} ").and_then(|rest| rest.strip_prefix(RESOLVED)) {
        Some(rest) if thread.resolved => Line::from(vec![
            Span::styled("\u{2576} ", grey),
            Span::styled(RESOLVED, resolved_style()),
            Span::styled(rest.to_owned(), grey),
        ]),
        _ => Line::styled(line, grey),
    }
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
    use ratatui::layout::Rect;

    use super::super::{Message, Thread};
    use super::{Line, RESOLVED, lines_of};
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
            start: None,
            resolved,
            folded: None,
            marks: Vec::new(),
            messages,
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
        assert!(lines[0].contains("\u{2713} Resolved"), "marked resolved while open: {lines:#?}");
        let nested = lines.iter().filter(|l| l.starts_with("\u{2502} \u{2502} \u{256d}")).count();
        assert_eq!(nested, 1, "the deepest thread is a box in a box in the box: {lines:#?}");
        assert!(lines.iter().any(|l| l.contains("Still open.")), "{lines:#?}");

        top.messages[0].threads[0].messages[0].threads[0].resolved = true;
        let lines = drawn(&top, 60);
        assert_eq!(lines.len(), 1, "everything resolved: one line");
        assert!(lines[0].to_string().contains("3 messages \u{b7} 2 sub-threads"), "{lines:#?}");
    }

    #[test]
    fn the_resolved_mark_takes_the_looks_good_colour_open_or_folded() {
        let open_under = thread("b", false, vec![message("You", "Still open.", Vec::new())]);
        let held_open = thread("a", true, vec![message("You", "The note.", vec![open_under])]);
        let folded = thread("c", true, vec![message("You", "Done.", Vec::new())]);
        for top in [&held_open, &folded] {
            let first = drawn(top, 60).into_iter().next().expect("a head or a folded line");
            let mark = first.spans.iter().find(|span| span.content == RESOLVED);
            assert_eq!(mark.and_then(|span| span.style.fg), Some(palette().looks_good), "{first}");
        }
    }
}
