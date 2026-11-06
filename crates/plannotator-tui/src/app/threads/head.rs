//! What a thread says of itself in one line, the same at every depth, for every kind and in
//! every state, by one code for the three places that say it: the head of its box, `▾` first;
//! its folded line, `▸` first, which says what the head says and what the box holds; and its
//! bubble's title in the rail. Its kind's glyph and its passage, then, each after a dot, the
//! indicators on the left, a rule, and the indicators on the right, the state last. The words
//! of the indicators, their colours (`mark_style`), their places and which of them give way
//! first on a narrow line (`Mark`) are decided here alone, and so are the counts of what a box
//! holds (`counts`). Each line comes with the columns that are its box's border, grey here,
//! which the box the person is in has drawn lit over them, in its state's colour (`lit_border`,
//! `rows::BoxRow::border`).

use std::ops::Range;

use plannotator_tui_schema::Kind;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::super::glyph;
use super::Thread;
use super::rows::{fit, one_line};
use crate::theme::palette;

/// Columns of the passage a box's head and its folded line keep before the indicators give way.
const MIN_QUOTE_WIDTH: usize = 16;

/// Columns of the passage a bubble's title keeps before the indicators give way: fewer than a
/// box's, since the box under the passage shows it whole and the rail is narrow.
const MIN_TITLE_QUOTE_WIDTH: usize = 6;

/// A Live thread, while it is open: a reply saved under it goes at once.
pub(super) const LIVE: &str = "\u{26a1} Live";

// The states of a comment, in the order it goes through them.

/// A comment just written that the journal has not taken yet: it has no box, its bubble says
/// it.
pub(super) const NOT_SAVED: &str = "Not saved";

/// A message of the person's not sent yet, beside "You" and, while it is their last in its box,
/// at the box's head.
pub(super) const DRAFT: &str = "Draft";

/// A send not yet in the agent's session.
pub(super) const QUEUED: &str = "Queued";

/// A message of the person's sent and waiting for its answer: what the journal calls delivered,
/// at its box's head; beside "You", any send not answered yet.
pub(super) const SENT: &str = "Sent";

/// A send the agent read.
pub(super) const READ: &str = "Read";

/// A send the agent's session works on.
pub(super) const WORKING: &str = "Working";

/// The person's last message in a box, once the agent answered after it, at the box's head.
pub(super) const REPLIED: &str = "Replied";

/// A resolved thread: its head says neither what became of its last send nor Live.
pub(super) const RESOLVED: &str = "Resolved";

/// A reply of the agent's the person has not seen: beside its author, and on the folded line
/// of a box holding it at any depth.
pub(super) const NEW: &str = "New";

/// A thread whose words are no longer found.
const ORPHANED: &str = "Orphaned";

/// What is drawn grey: the indicators that have no colour of their own (the round,
/// `Orphaned`, `Not saved`, `Queued`, `Read`, `Working`), the counts, the dots between them.
pub(in crate::app) const GREY: Style = Style::new().fg(Color::DarkGray);

/// The border of every box at rest, its head's and its folded line's rule with it, at every
/// depth, of every kind and in every state: grey, as a bubble's in the rail that is not lit.
/// The box the person is in, the current note's, is drawn in its state's colour over it
/// (`lit_border`).
pub(super) const BORDER: Style = Style::new().fg(Color::DarkGray);

/// An indicator a thread's head says, its folded line and its bubble's title alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Mark {
    /// Its words are no longer found.
    Orphaned,
    /// Live, while it is open.
    Live,
    /// A reply in its box, at any depth, the person has not seen: on its folded line and in
    /// its bubble's title.
    New,
    /// The round its comment was sent in; a draft has none.
    Round(u64),
    /// Its box's state, the person's last message's (`journal::box_state`): `Not saved` (its
    /// bubble alone says it), `Draft`, what became of its send, `Resolved`.
    State(&'static str),
}

impl Mark {
    fn word(&self) -> String {
        match self {
            Self::Orphaned => ORPHANED.to_owned(),
            Self::Live => LIVE.to_owned(),
            Self::New => NEW.to_owned(),
            Self::Round(round) => format!("Round {round}"),
            Self::State(state) => (*state).to_owned(),
        }
    }

    /// Where it is said, in this order: on the left, after the passage, `Orphaned` then
    /// `⚡ Live`; on the right, `New`, the round, then the state, last.
    fn place(&self) -> u8 {
        match self {
            Self::Orphaned => 0,
            Self::Live => 1,
            Self::New => 2,
            Self::Round(_) => 3,
            Self::State(_) => 4,
        }
    }

    fn on_left(&self) -> bool {
        self.place() < Self::New.place()
    }

    /// When it gives way on a line too narrow for every indicator, the lowest first: the
    /// round first, the state last, so a narrow line still says where its thread is.
    fn gives_way(&self) -> u8 {
        match self {
            Self::Round(_) => 0,
            Self::Orphaned => 1,
            Self::New => 2,
            Self::Live => 3,
            Self::State(_) => 4,
        }
    }
}

/// The state `marks` say, their last word: the colour of a box and of its bubble lit.
pub(super) fn state_of(marks: &[Mark]) -> Option<&'static str> {
    marks.iter().find_map(|mark| match mark {
        Mark::State(state) => Some(*state),
        _ => None,
    })
}

/// The colour of an indicator that has one.
fn colour(word: &str) -> Option<Color> {
    let palette = palette();
    Some(match word {
        LIVE => palette.live,
        DRAFT => palette.draft,
        SENT => palette.sent,
        REPLIED => palette.replied,
        RESOLVED => palette.resolved,
        NEW => palette.new,
        _ => return None,
    })
}

/// How an indicator is drawn wherever a box, its folded line, its bubble's title or an author
/// line says it: Live in the lightning's yellow, a draft light blue, sent orange, replied
/// violet, new turquoise, each bold; resolved green, not bold; the others grey.
pub(super) fn mark_style(word: &str) -> Style {
    match colour(word) {
        Some(colour) if word == RESOLVED => Style::new().fg(colour),
        Some(colour) => Style::new().fg(colour).add_modifier(Modifier::BOLD),
        None => GREY,
    }
}

/// The border of the box the person is in, the innermost, and of its bubble lit in the rail:
/// its state's colour (`state_of`), grey for a state drawn grey (`Not saved`, `Queued`, `Read`,
/// `Working`) and for none.
pub(super) fn lit_border(state: Option<&str>) -> Style {
    Style::new().fg(state.and_then(colour).unwrap_or(Color::DarkGray))
}

/// What a box holds, as its folded line and its bubble count it, in this order: its messages
/// (its comment and the replies under it, not its sub-threads'), then, only when it has some,
/// its sub-threads and its comment's images.
pub(super) fn counts(messages: usize, sub_threads: usize, images: usize) -> Vec<String> {
    let count = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    std::iter::once(count(messages, "message"))
        .chain((sub_threads > 0).then(|| count(sub_threads, "sub-thread")))
        .chain((images > 0).then(|| count(images, "image")))
        .collect()
}

/// Where a thread's line is drawn, and how.
struct Shape {
    /// What opens it, its box's border.
    lead: &'static str,
    /// Whether its passage is in italics: at its head and in its bubble's title, not on its
    /// folded line. The glyph and the passage are never its box's border: grey, they keep their
    /// colour when the box is lit with its state's.
    passage_italic: bool,
    /// What closes it, its box's border.
    end: &'static str,
    /// Columns of the passage kept before the indicators give way (`Mark::gives_way`).
    min_quote: usize,
    /// Whether it says the round: a box's head and its folded line do, a bubble's title does
    /// not.
    round: bool,
}

/// `╭ ▾ ✗ "passage" · Orphaned · ⚡ Live ──── Round 2 · Sent ╮`: `▾`, where `left` or a click
/// folds the box, its kind's glyph, the passage cut to fit, the indicators. With the columns of
/// its border: all but the glyph, the passage and the indicators, which keep their own colours.
pub(super) fn head(thread: &Thread, width: usize) -> (Line<'static>, Vec<Range<usize>>) {
    let shape = Shape {
        lead: "\u{256d} \u{25be} ",
        passage_italic: true,
        end: "\u{256e}",
        min_quote: MIN_QUOTE_WIDTH,
        round: true,
    };
    line(&shape, (thread.kind, &thread.quote), &thread.marks, &[], width, BORDER)
}

/// A folded thread in one line, saying what the head of its box says, then what the box holds:
/// `▸ 💬 "passage" · ⚡ Live · 3 messages · 1 sub-thread ──── New · Round 2 · Sent ─╴` (`New`
/// while a reply in it, at any depth, is), which `right` or a click draws open there; `╶`
/// first for a box too narrow to draw, which does not open. On a narrow line the indicators
/// give way, the counts stay. `▸` and the rule are its box's border, with their columns.
pub(super) fn folded(thread: &Thread, width: usize) -> (Line<'static>, Vec<Range<usize>>) {
    let lead = if thread.is_folded() { "\u{25b8} " } else { "\u{2576} " };
    let shape = Shape {
        lead,
        passage_italic: false,
        end: "\u{2500}\u{2574}",
        min_quote: MIN_QUOTE_WIDTH,
        round: true,
    };
    line(&shape, (thread.kind, &thread.quote), &thread.marks_and_new(), &thread.counts(), width, BORDER)
}

/// A bubble's top edge in the rail, `width` columns, corners and all, as the head of its box
/// but for the round, `New` first on the right as on its folded line:
/// `╭ 💬 "Deploy…" · ⚡ Live ──── New · Sent ╮`, its passage cut first, drawn in `border`.
pub(super) fn title(kind: Kind, quote: &str, marks: &[Mark], width: usize, border: Style) -> Line<'static> {
    let shape = Shape {
        lead: "\u{256d} ",
        passage_italic: true,
        end: "\u{256e}",
        min_quote: MIN_TITLE_QUOTE_WIDTH,
        round: false,
    };
    line(&shape, (kind, quote), marks, &[], width, border).0
}

/// A thread's line, `width` columns: `shape`'s lead, the kind's glyph, the passage cut to fit
/// between quotes, the indicators on the left each after a dot, then `counts` (a folded box's),
/// a rule, the indicators on the right (the round where `shape` says it), `shape`'s end. When
/// the line is too narrow for every indicator, they give way one by one (`Mark::gives_way`)
/// before the passage keeps fewer than `shape.min_quote` columns (or its own width); the counts
/// never do, and a line still too wide is cut. With the columns that are its box's border,
/// drawn in `border`.
fn line(
    shape: &Shape,
    (kind, quote): (Kind, &str),
    marks: &[Mark],
    counts: &[String],
    width: usize,
    border: Style,
) -> (Line<'static>, Vec<Range<usize>>) {
    let passage = one_line(quote);
    let glyph = format!("{} ", glyph(kind));
    let mut kept: Vec<&Mark> =
        marks.iter().filter(|mark| shape.round || !matches!(mark, Mark::Round(_))).collect();
    kept.sort_by_key(|mark| mark.place());
    let words = |kept: &[&Mark], left: bool| -> Vec<String> {
        kept.iter().filter(|mark| mark.on_left() == left).map(|mark| mark.word()).collect()
    };
    // Each word on the left after a dot; the ones on the right between a space each side.
    let after_dots = |words: &[String]| words.iter().map(|word| word.width() + 3).sum::<usize>();
    let between_spaces =
        |words: &[String]| if words.is_empty() { 0 } else { words.join(" \u{b7} ").width() + 2 };
    // All but the passage: its quotes, the space before the rule and one column of it at least.
    let around = |kept: &[&Mark]| {
        shape.lead.width()
            + glyph.width()
            + 2
            + after_dots(&words(kept, true))
            + after_dots(counts)
            + 2
            + between_spaces(&words(kept, false))
            + shape.end.width()
    };
    let wanted = passage.width().min(shape.min_quote);
    while around(&kept) + wanted > width {
        let first = kept.iter().enumerate().min_by_key(|(_, mark)| mark.gives_way()).map(|(at, _)| at);
        let Some(first) = first else { break };
        kept.remove(first);
    }
    let quote = format!("\"{}\"", fit(&passage, width.saturating_sub(around(&kept))));
    let fill = width.saturating_sub(around(&kept) + quote.width() - 3).max(1);
    // The quoted passage never takes the state's colour of a lit box: it stays grey.
    let quote_style = if shape.passage_italic { GREY.add_modifier(Modifier::ITALIC) } else { GREY };
    let mut parts = vec![
        (Span::styled(shape.lead, border), true),
        (Span::styled(glyph, GREY), false),
        (Span::styled(quote, quote_style), false),
    ];
    for word in words(&kept, true) {
        let style = mark_style(&word);
        parts.extend([(Span::styled(" \u{b7} ", GREY), false), (Span::styled(word, style), false)]);
    }
    for count in counts {
        parts.extend([(Span::styled(" \u{b7} ", GREY), false), (Span::styled(count.clone(), GREY), false)]);
    }
    parts.push((Span::styled(format!(" {}", "\u{2500}".repeat(fill)), border), true));
    let right = words(&kept, false);
    for (index, word) in right.iter().enumerate() {
        let before = if index == 0 { " " } else { " \u{b7} " };
        parts.extend([
            (Span::styled(before, GREY), false),
            (Span::styled(word.clone(), mark_style(word)), false),
        ]);
    }
    if !right.is_empty() {
        parts.push((Span::styled(" ", GREY), false));
    }
    parts.push((Span::styled(shape.end, border), true));
    let parts = cut(parts, width);
    let mut columns: Vec<Range<usize>> = Vec::new();
    let mut at = 0;
    for (span, of_border) in &parts {
        let next = at + span.width();
        if *of_border {
            match columns.last_mut() {
                Some(last) if last.end == at => last.end = next,
                _ => columns.push(at..next),
            }
        }
        at = next;
    }
    (Line::from(parts.into_iter().map(|(span, _)| span).collect::<Vec<_>>()), columns)
}

/// `parts` cut to `width` columns, the last one kept ending in `…` when they are cut.
fn cut(parts: Vec<(Span<'static>, bool)>, width: usize) -> Vec<(Span<'static>, bool)> {
    if parts.iter().map(|(span, _)| span.width()).sum::<usize>() <= width {
        return parts;
    }
    let mut room = width;
    let mut kept = Vec::new();
    for (span, of_border) in parts {
        // A part that fits with a column to spare for the `…` stays whole.
        if span.width() < room {
            room -= span.width();
            kept.push((span, of_border));
            continue;
        }
        let mut text = String::new();
        for ch in span.content.chars() {
            if text.width() + ch.width().unwrap_or(0) + 1 > room {
                break;
            }
            text.push(ch);
        }
        if room > 0 {
            text.push('\u{2026}');
        }
        kept.push((Span::styled(text, span.style), of_border));
        break;
    }
    kept
}
