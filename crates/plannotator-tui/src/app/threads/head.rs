//! What a thread says of itself in one line, the same at every depth, for every kind and in
//! every state: the head of its box (its kind's glyph, its passage, its state and marks, its
//! id), `▾` first, or, folded, its folded line, `▸` first, which says what the head says and
//! what the box holds.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::super::draw::{accent, short_id};
use super::super::glyph;
use super::Thread;
use super::journal::NEW;
use super::rows::{fit, one_line};
use crate::theme::palette;

/// Columns of the passage a folded line keeps before it gives up the note's id.
const MIN_QUOTE_WIDTH: usize = 16;

/// The mark of a resolved thread: at its box's head, on its folded line, in its bubble's title.
pub(in crate::app) const RESOLVED: &str = "\u{2713} Resolved";

/// How the resolved mark is drawn: in the looks-good colour, so it stands apart from the grey
/// of the rest of the line.
pub(in crate::app) fn resolved_style() -> Style {
    Style::new().fg(palette().looks_good).add_modifier(Modifier::BOLD)
}

/// The border of a resolved thread, its box's or its folded line's rule: the resolved mark's
/// colour, at every depth; a thread under it that is still open keeps its own.
fn resolved_border() -> Style {
    Style::new().fg(palette().looks_good)
}

/// The border of a thread's box, and its folded line's mark and rule: green once resolved
/// (`resolved_border`), else its kind's colour.
pub(super) fn border(thread: &Thread) -> Style {
    if thread.resolved { resolved_border() } else { Style::new().fg(accent(thread.kind)) }
}

/// `╭ ▾ ✗ "passage" ──── Deletion · Replied · Live · Round 2 · 3e05f ╮`: `▾`, where `left` or a
/// click folds the box, the passage cut to fit, the state, the marks and the tail of the note's
/// id, which its bubble's title shows too. In a narrow box the marks go first, the last first,
/// then the state, then the id, before the passage gets too short.
pub(super) fn head(thread: &Thread, width: usize, border: Style) -> Line<'static> {
    let lead = format!("\u{256d} \u{25be} {} ", glyph(thread.kind));
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

/// A folded thread in one line, saying what the head of its box says, then what the box holds:
/// `▸ 💬 "passage" · ✓ Resolved · Round 2 · 3 messages · 1 sub-thread · 3e05f ─╴`,
/// `▸ ✗ "passage" · Deletion · Replied · Round 2 · New · 2 messages · 3e05f ─╴` (`New` while a
/// reply in it, at any depth, is), which `right` or a click draws open there; `╶` first for a
/// box too narrow to draw, which does not open. `▸` and the rule take its box's border.
pub(super) fn folded(thread: &Thread, width: usize) -> Line<'static> {
    let (messages, threads) = thread.size();
    let start = if thread.is_folded() { "\u{25b8} " } else { "\u{2576} " };
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let counts = match threads {
        0 => format!("{messages} message{}", plural(messages)),
        _ => format!("{messages} message{} \u{b7} {threads} sub-thread{}", plural(messages), plural(threads)),
    };
    let lead = format!("{start}{} ", glyph(thread.kind));
    let passage = one_line(&thread.quote);
    let new = thread.holds_new().then_some(NEW);
    // The head's chips in the head's order, the resolved mark first, then what is new.
    let resolved = thread.resolved.then_some(RESOLVED);
    let marks: String = resolved
        .into_iter()
        .chain(thread.marks.iter().map(String::as_str))
        .chain(new)
        .flat_map(|mark| [" \u{b7} ", mark])
        .collect();
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
    let Some(rest) = line.strip_prefix(start) else { return Line::styled(line, grey) };
    // The mark that opens the line and the rule that closes it are the folded box's border.
    let border = border(thread);
    let mut spans = vec![Span::styled(start, border)];
    let words = rest.trim_end_matches(['\u{2500}', '\u{2574}']);
    // The resolved mark, right after the passage, in its colour as at the head.
    let after_quote = lead.len() - start.len() + quote.len();
    let mark = thread
        .resolved
        .then(|| words.get(after_quote..)?.find(RESOLVED).map(|at| after_quote + at))
        .flatten();
    match mark {
        Some(at) => {
            spans.push(Span::styled(words[..at].to_owned(), grey));
            spans.push(Span::styled(RESOLVED, resolved_style()));
            spans.push(Span::styled(words[at + RESOLVED.len()..].to_owned(), grey));
        }
        None => spans.push(Span::styled(words.to_owned(), grey)),
    }
    spans.push(Span::styled(rest[words.len()..].to_owned(), border));
    Line::from(spans)
}
