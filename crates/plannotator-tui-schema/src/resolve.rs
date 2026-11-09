//! Re-finding an anchor in a document that may have changed since the anchor was made.
//!
//! An anchor with a place (`Extras::place`, its lines) is found as crit finds a comment
//! (`crate::place`): its lines followed through the text's changes, checked, else looked for
//! elsewhere, else kept as followed; its quote is then searched in that place only, and a
//! place without it is the note's place, marked (`Resolution::Place`).
//!
//! An anchor without one is quote-primary, like the W3C `TextQuoteSelector` and Plannotator's
//! own resolver: the raw source text is searched for the quoted range; among several
//! occurrences the one whose surrounding context matches wins, then the one nearest the block
//! hint. A quote that is nowhere in the document is an orphan — never silently bound to the
//! wrong text.

use std::ops::Range;

use crate::anchor::{Anchor, Extras, floor_char_boundary};
use crate::place::{Base, Place, line_range, place_base, track_place};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The anchor's text is here, as a byte range into the current source.
    Range(Range<usize>),
    /// The anchor's quote is not in its place, the bytes of its lines (`range`), which is
    /// marked: its words were found there or elsewhere (`found`), else it is Orphaned, at the
    /// place its lines were followed to.
    Place { range: Range<usize>, found: bool },
    /// The anchor's text is no longer in the document, and it has no place.
    Orphan,
    /// A note on the whole document, with no words of its own (a general comment): its box
    /// goes first, before the title.
    Document,
}

/// Resolve `anchor` against `source`. `block_of` maps a byte offset to the index of the
/// top-level block containing it, so the block hint can break ties; pass `|_| None` when
/// blocks are unknown. `previous` is the text shown before `source`, when the client holds
/// it: an anchor's place counted in it follows its lines from it.
pub fn resolve(
    anchor: &Anchor,
    source: &str,
    block_of: impl Fn(usize) -> Option<usize>,
    previous: Option<&str>,
) -> Resolution {
    if anchor.on_document() {
        return Resolution::Document;
    }
    if let Some((extras, place)) =
        anchor.plannotator_tui.as_ref().and_then(|e| e.place.as_ref().map(|place| (e, place)))
    {
        return resolve_place(anchor, extras, place, source, &block_of, previous);
    }
    let Some(extras) = anchor.plannotator_tui.as_ref().filter(|e| !e.quote.is_empty()) else {
        // A foreign anchor carries only rendered text; try it as-is against the source.
        return first_occurrence(source, anchor.rendered()).map_or(Resolution::Orphan, Resolution::Range);
    };
    if let Some(range) = extras.source.as_ref().filter(|r| range_is_current(source, r.start, r.end, extras)) {
        return Resolution::Range(range.start..range.end);
    }
    let quote = extras.quote.as_str();
    if quote.trim().is_empty() {
        return Resolution::Orphan;
    }
    best_occurrence(source, quote, extras, &block_of)
        .or_else(|| {
            let trimmed = quote.trim();
            (trimmed != quote).then(|| best_occurrence(source, trimmed, extras, &block_of)).flatten()
        })
        .map_or(Resolution::Orphan, Resolution::Range)
}

/// An anchor with a place: its stored range when it still holds its quote between its context;
/// else its lines followed (from `previous` when they are counted in it), checked, else looked
/// for elsewhere; its quote then searched in that place, the place itself when it is not there.
fn resolve_place(
    anchor: &Anchor,
    extras: &Extras,
    place: &Place,
    source: &str,
    block_of: &impl Fn(usize) -> Option<usize>,
    previous: Option<&str>,
) -> Resolution {
    if !extras.quote.is_empty()
        && let Some(range) =
            extras.source.as_ref().filter(|r| range_is_current(source, r.start, r.end, extras))
    {
        return Resolution::Range(range.start..range.end);
    }
    let from = match place_base(&place.text_hash, source, previous) {
        Base::Previous => previous,
        Base::Current | Base::Unknown => None,
    };
    let quote = if extras.quote.is_empty() { anchor.rendered() } else { extras.quote.as_str() };
    let tracked = track_place(place.lines, &place.lines_text, quote, source, from);
    let area = line_range(source, tracked.lines);
    let raw = extras.quote.as_str();
    if !raw.trim().is_empty() {
        let found = best_occurrence_in(source, raw, extras, block_of, &area).or_else(|| {
            let trimmed = raw.trim();
            (trimmed != raw).then(|| best_occurrence_in(source, trimmed, extras, block_of, &area)).flatten()
        });
        if let Some(range) = found {
            return Resolution::Range(range);
        }
    }
    Resolution::Place { range: area, found: tracked.found }
}

/// True when the stored range still holds the stored quote between its stored context.
fn range_is_current(source: &str, start: usize, end: usize, extras: &Extras) -> bool {
    start <= end
        && end <= source.len()
        && source.is_char_boundary(start)
        && source.is_char_boundary(end)
        && source[start..end] == extras.quote
        && source[..start].ends_with(&extras.prefix)
        && source[end..].starts_with(&extras.suffix)
}

fn first_occurrence(source: &str, needle: &str) -> Option<Range<usize>> {
    if needle.trim().is_empty() {
        return None;
    }
    source.find(needle).map(|start| start..start + needle.len())
}

/// Among all occurrences of `needle`, prefer matching context, then proximity to the hint.
fn best_occurrence(
    source: &str,
    needle: &str,
    extras: &Extras,
    block_of: &impl Fn(usize) -> Option<usize>,
) -> Option<Range<usize>> {
    best_occurrence_in(source, needle, extras, block_of, &(0..source.len()))
}

/// `best_occurrence` among the occurrences that lie within bytes `within` of `source`.
fn best_occurrence_in(
    source: &str,
    needle: &str,
    extras: &Extras,
    block_of: &impl Fn(usize) -> Option<usize>,
    within: &Range<usize>,
) -> Option<Range<usize>> {
    let mut best: Option<((u8, usize), Range<usize>)> = None;
    let mut from = floor_char_boundary(source, within.start);
    while let Some(at) = source[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        if end > within.end {
            break;
        }
        let score = u8::from(source[..start].ends_with(&extras.prefix))
            + u8::from(source[end..].starts_with(&extras.suffix));
        let distance = match (extras.block, block_of(start)) {
            (Some(hint), Some(block)) => hint.abs_diff(block),
            _ => 0,
        };
        let key = (score, usize::MAX - distance);
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, start..end));
        }
        from = floor_char_boundary(source, start + needle.len().max(1));
        if from >= source.len() {
            break;
        }
    }
    best.map(|(_, range)| range)
}

/// Predict whether the Workspaces web client will highlight this anchor over `rendered`,
/// the document's rendered text stream (block texts joined with no separator). Mirrors the
/// client's lookup: exact substring first, then with runs of whitespace collapsed.
pub fn web_will_match(anchor: &Anchor, rendered: &str) -> bool {
    let needle = anchor.rendered();
    if needle.is_empty() {
        return false;
    }
    rendered.contains(needle) || collapse_ws(rendered).contains(&collapse_ws(needle))
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::anchor::{Kind, SourceRange};

    fn anchor_on(source: &str, quote: &str, block: Option<usize>) -> Anchor {
        let start = source.find(quote).expect("quote present");
        let range = SourceRange { start, end: start + quote.len(), version: "v1".into() };
        Anchor::new(quote, source, range, Kind::Comment, block)
    }

    fn no_blocks(_: usize) -> Option<usize> {
        None
    }

    #[test]
    fn unchanged_document_resolves_to_the_stored_range() {
        let source = "# A\n\nfirst\n\nsecond thing\n";
        let anchor = anchor_on(source, "second", None);
        assert_eq!(resolve(&anchor, source, no_blocks, None), Resolution::Range(12..18));
    }

    #[test]
    fn survives_text_inserted_above() {
        let source = "# A\n\nfirst\n\nsecond thing\n";
        let anchor = anchor_on(source, "second", None);
        let edited = "# A\n\ninserted paragraph\n\nfirst\n\nsecond thing\n";
        let expected = edited.find("second").expect("present");
        assert_eq!(resolve(&anchor, edited, no_blocks, None), Resolution::Range(expected..expected + 6));
    }

    #[test]
    fn orphans_when_the_text_is_gone() {
        let source = "one\n\ntwo\n";
        let anchor = anchor_on(source, "two", None);
        assert_eq!(resolve(&anchor, "one\n\nthree\n", no_blocks, None), Resolution::Orphan);
    }

    #[test]
    fn duplicates_disambiguate_by_context() {
        let source = "alpha same\n\nbeta same\n";
        let second = source.rfind("same").expect("present");
        let range = SourceRange { start: second, end: second + 4, version: "v1".into() };
        let anchor = Anchor::new("same", source, range, Kind::Comment, None);
        let edited = "same\n\nalpha same\n\nbeta same\n";
        let expected = edited.rfind("same").expect("present");
        assert_eq!(resolve(&anchor, edited, no_blocks, None), Resolution::Range(expected..expected + 4));
    }

    #[test]
    fn a_rewritten_range_with_intact_context_is_not_trusted() {
        // The old range now holds different text: the stored quote is the authority.
        let source = "# A\n\nfirst\n\nsecond thing\n";
        let anchor = anchor_on(source, "second", None);
        let edited = "# A\n\nfirst\n\nSECOND thing\n";
        assert_eq!(resolve(&anchor, edited, no_blocks, None), Resolution::Orphan);
    }

    #[test]
    fn foreign_anchor_resolves_by_rendered_text() {
        let anchor: Anchor =
            serde_json::from_value(serde_json::json!({"originalText": "brown fox"})).expect("parses");
        assert_eq!(resolve(&anchor, "the quick brown fox", no_blocks, None), Resolution::Range(10..19));
    }

    #[test]
    fn web_match_prediction_follows_the_client_rules() {
        let doc = "these words.The second paragraph";
        let hit = Anchor { original_text: "words.The".into(), ..Anchor::default() };
        let miss = Anchor { original_text: "words.\n\nThe".into(), ..Anchor::default() };
        let collapsed = Anchor { original_text: "second  paragraph".into(), ..Anchor::default() };
        assert!(web_will_match(&hit, doc));
        assert!(!web_will_match(&miss, doc));
        assert!(web_will_match(&collapsed, doc));
    }
}
