//! Finding a comment's words in a document as drawn. The Inbox keeps a comment's quote as
//! the person selected it, in the words as drawn (`originalText`); its window finds it
//! there, every run of whitespace read as one. These functions do the same over a
//! document's rendered characters (`DocLayout::rendered_chars`), each paired with the
//! source byte it shows, so that a quote found maps back to a source range.

use std::ops::Range;

/// The characters of `chars` with each run of whitespace read as one space and leading
/// whitespace dropped, with each kept character's index in `chars`.
fn normalized(chars: &[(char, usize)]) -> (Vec<char>, Vec<usize>) {
    let mut flat = Vec::new();
    let mut at = Vec::new();
    let mut space = false;
    for (index, (ch, _)) in chars.iter().enumerate() {
        if ch.is_whitespace() {
            space = !flat.is_empty();
            continue;
        }
        if space {
            flat.push(' ');
            at.push(index.saturating_sub(1));
            space = false;
        }
        flat.push(*ch);
        at.push(index);
    }
    (flat, at)
}

/// `quote` as a passage compares it: whitespace runs as one space, trimmed.
fn flat_quote(quote: &str) -> Vec<char> {
    quote.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect()
}

/// Where each occurrence of `quote` starts in the normalized characters.
fn starts(flat: &[char], quote: &[char]) -> Vec<usize> {
    if quote.is_empty() || quote.len() > flat.len() {
        return Vec::new();
    }
    (0..=flat.len() - quote.len()).filter(|&from| flat.get(from..from + quote.len()) == Some(quote)).collect()
}

/// The source range of the `nth` occurrence (from 0) of `quote` in `chars`, a document's
/// rendered characters with their source offsets; `source` is that document's text.
pub(crate) fn find(chars: &[(char, usize)], source: &str, quote: &str, nth: usize) -> Option<Range<usize>> {
    let (flat, at) = normalized(chars);
    let quote = flat_quote(quote);
    let from = *starts(&flat, &quote).get(nth)?;
    let (_, start) = *chars.get(*at.get(from)?)?;
    let (_, last) = *chars.get(*at.get(from + quote.len() - 1)?)?;
    let width = source.get(last..).and_then(|rest| rest.chars().next()).map_or(1, char::len_utf8);
    Some(start..last + width)
}

/// The source range of the occurrence of `quote` nearest to where the Inbox says its passage
/// starts, `offset` UTF-16 units into the words with whitespace runs read as one (a
/// sub-thread's `quote_offset`): the Inbox counts in its own words, so the nearest one wins
/// rather than an exact match.
pub(crate) fn find_near(
    chars: &[(char, usize)],
    source: &str,
    quote: &str,
    offset: u64,
) -> Option<Range<usize>> {
    let (flat, _) = normalized(chars);
    let wanted = flat_quote(quote);
    let units = |from: usize| flat.iter().take(from).map(|ch| ch.len_utf16() as u64).sum::<u64>();
    let starts = starts(&flat, &wanted);
    let nth = (0..starts.len())
        .min_by_key(|&nth| starts.get(nth).map_or(u64::MAX, |&from| units(from).abs_diff(offset)))?;
    find(chars, source, quote, nth)
}

/// How many occurrences of `quote` in `chars` start before source offset `before`: which
/// occurrence a selection starting there is.
pub(crate) fn occurrence(chars: &[(char, usize)], quote: &str, before: usize) -> usize {
    let (flat, at) = normalized(chars);
    let quote = flat_quote(quote);
    let starts_before = |from: &usize| {
        at.get(*from).and_then(|index| chars.get(*index)).is_some_and(|(_, offset)| *offset < before)
    };
    starts(&flat, &quote).iter().filter(|from| starts_before(from)).count()
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::{find, find_near, occurrence};

    /// Rendered characters as a layout pairs them: here, the source with `**` markup left out.
    fn drawn(source: &str) -> Vec<(char, usize)> {
        source.char_indices().filter(|(_, ch)| *ch != '*').map(|(offset, ch)| (ch, offset)).collect()
    }

    #[test]
    fn a_quote_is_found_in_the_words_as_drawn_and_maps_back_to_its_source() {
        let source = "Ship the **login page** by Friday.";
        let chars = drawn(source);
        let range = find(&chars, source, "login page by", 0).expect("found across the markup");
        assert_eq!(source.get(range), Some("login page** by"));
        assert_eq!(find(&chars, source, "logout", 0), None);
    }

    #[test]
    fn whitespace_runs_match_as_one_and_later_occurrences_are_counted() {
        let source = "one  two\none two";
        let chars = drawn(source);
        let second = find(&chars, source, "one two", 1).expect("the second one");
        assert_eq!(second, 9..16);
        assert_eq!(occurrence(&chars, "one two", second.start), 1, "a selection there is the second");
        assert_eq!(occurrence(&chars, "one two", 0), 0);
    }

    #[test]
    fn a_sub_threads_passage_is_the_occurrence_nearest_where_the_inbox_says_it_starts() {
        let source = "the panel, then the panel";
        let chars = drawn(source);
        assert_eq!(find_near(&chars, source, "panel", 0), Some(4..9));
        assert_eq!(find_near(&chars, source, "panel", 19), Some(20..25));
        assert_eq!(find_near(&chars, source, "slider", 4), None);
    }
}
