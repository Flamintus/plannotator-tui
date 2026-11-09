//! A thread's place in a text that changes: its lines, followed from one text to the next as
//! crit does (`internal/session/watch.go`), then its words looked for at that place.
//!
//! - The lines follow a line-by-line difference between the text they were counted in and the
//!   new one ([`line_map`]): a kept line goes where it went, a removed one to the next new line.
//! - The words are then checked there (`lines_text`, the lines' text when the comment was made,
//!   never refreshed): the same, or close enough ([`anchor_similar`]), or the quote among them.
//! - Else they are looked for elsewhere: the lines' text, then the quote, the nearest
//!   occurrence winning.
//! - Else the thread keeps the place it followed, its words not found there (Orphaned): the
//!   place is still where its box goes and what is marked.
//!
//! `text_hash` names the text a place's lines are counted in, so a client only follows lines
//! through a difference from the text they were counted in, and never twice. Plannotator's
//! `packages/core/place.ts` is the same rule, case for case.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// What a note keeps of its place: its lines (first and last, 1-based, inclusive), their text
/// when it was made, and the fingerprint of the text they are counted in.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Place {
    pub lines: [usize; 2],
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lines_text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text_hash: String,
}

impl Place {
    /// The place of bytes `range` of `text`: its lines, their text, the text's fingerprint.
    pub fn of(text: &str, range: &Range<usize>) -> Self {
        let first = line_at(text, range.start);
        let last = line_at(text, range.end.saturating_sub(1).max(range.start));
        let lines = [first, last.max(first)];
        Self { lines, lines_text: lines_text(text, lines), text_hash: text_hash(text) }
    }
}

/// Where a thread is in the current text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tracked {
    pub lines: [usize; 2],
    /// Its words were found: at the place followed, or elsewhere. False: Orphaned, at the place
    /// followed.
    pub found: bool,
    /// Its quote is among the place's lines (what is highlighted); else the place is marked.
    pub quote_here: bool,
}

/// Which text a place's lines are counted in, of the ones a client holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// The current text: the lines are taken as they are, then checked.
    Current,
    /// The previous text the client holds: the client follows the lines, and writes where
    /// they went.
    Previous,
    /// Neither: the lines are only checked, nothing written.
    Unknown,
}

/// Beyond this many cells, a difference's middle is read as replaced whole.
const MAX_CELLS: usize = 4_000_000;

/// Beyond this many pairs of a removed and an added line, a changed run is not paired.
const MAX_PAIRS: usize = 400;

/// The text's fingerprint: FNV-1a, 32 bits, over its UTF-8 bytes, in hexadecimal.
pub fn text_hash(text: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// The text's lines, split on `\n` (none for an empty text).
pub fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() { Vec::new() } else { text.split('\n').collect() }
}

/// The text of `lines` in `text`, joined with `\n`; empty when they are not in it.
pub fn lines_text(text: &str, [start, end]: [usize; 2]) -> String {
    let all = split_lines(text);
    if start < 1 || end < start || start > all.len() {
        return String::new();
    }
    all.get(start - 1..end.min(all.len())).map_or_else(String::new, |lines| lines.join("\n"))
}

/// The bytes of `text` lines `lines` span, their last newline left out.
pub fn line_range(text: &str, [start, end]: [usize; 2]) -> Range<usize> {
    let from = line_start(text, start);
    let to = line_start(text, end + 1);
    let to = if to > from && text.as_bytes().get(to - 1) == Some(&b'\n') { to - 1 } else { to };
    from..to.max(from)
}

/// The line (1-based) holding byte `index` of `text`.
pub fn line_at(text: &str, index: usize) -> usize {
    1 + text.as_bytes().iter().take(index.min(text.len())).filter(|byte| **byte == b'\n').count()
}

/// Where line `line` (1-based) starts in `text`; the text's end past its last line.
fn line_start(text: &str, line: usize) -> usize {
    let mut at = 0;
    for _ in 1..line {
        match text.get(at..).and_then(|rest| rest.find('\n')) {
            Some(next) => at += next + 1,
            None => return text.len(),
        }
    }
    at
}

/// Where each line of `previous` went in `current`: index i (0-based) holds the 1-based new
/// line, the next new line for a removed one, 0 for one removed at the end. Within a run of
/// changed lines, a removed line goes to the added line most like it, when one is at least half
/// alike: a line retouched where lines were added before it goes to its new version.
pub fn line_map(previous: &[&str], current: &[&str]) -> Vec<usize> {
    let (m, n) = (previous.len(), current.len());
    let mut map = vec![0; m];
    let mut pre = 0;
    while pre < m && pre < n && previous.get(pre) == current.get(pre) {
        set(&mut map, pre, pre + 1);
        pre += 1;
    }
    let mut suf = 0;
    while suf < m - pre && suf < n - pre && previous.get(m - 1 - suf) == current.get(n - 1 - suf) {
        set(&mut map, m - 1 - suf, n - suf);
        suf += 1;
    }
    let (old_count, new_count) = (m - pre - suf, n - pre - suf);
    if old_count == 0 {
        return map;
    }
    let next = |j: usize| if pre + j < n { pre + j + 1 } else { 0 };
    if new_count == 0 || (old_count + 1) * (new_count + 1) > MAX_CELLS {
        for i in 0..old_count {
            set(&mut map, pre + i, next(0));
        }
        let removed: Vec<usize> = (pre..pre + old_count).collect();
        let added: Vec<usize> = (pre..pre + new_count).collect();
        pair_changed(&mut map, previous, current, &removed, &added);
        return map;
    }
    // lcs[i * width + j]: the longest common run of old[i..] and new[j..], over the middle.
    let width = new_count + 1;
    let mut lcs = vec![0_u32; (old_count + 1) * width];
    let cell = |lcs: &[u32], i: usize, j: usize| lcs.get(i * width + j).copied().unwrap_or(0);
    let same = |i: usize, j: usize| previous.get(pre + i) == current.get(pre + j);
    for i in (0..old_count).rev() {
        for j in (0..new_count).rev() {
            let value = if same(i, j) {
                cell(&lcs, i + 1, j + 1) + 1
            } else {
                cell(&lcs, i + 1, j).max(cell(&lcs, i, j + 1))
            };
            if let Some(slot) = lcs.get_mut(i * width + j) {
                *slot = value;
            }
        }
    }
    let (mut i, mut j) = (0, 0);
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    while i < old_count {
        if j < new_count && same(i, j) {
            pair_changed(&mut map, previous, current, &removed, &added);
            removed.clear();
            added.clear();
            set(&mut map, pre + i, pre + j + 1);
            i += 1;
            j += 1;
        } else if j >= new_count || cell(&lcs, i + 1, j) >= cell(&lcs, i, j + 1) {
            set(&mut map, pre + i, next(j));
            removed.push(pre + i);
            i += 1;
        } else {
            added.push(pre + j);
            j += 1;
        }
    }
    added.extend(pre + j..pre + new_count);
    pair_changed(&mut map, previous, current, &removed, &added);
    map
}

/// Within a run of changed lines, each removed line (`removed`, indices of `previous`) goes to
/// the added line (`added`, indices of `current`) most like it, the first on a tie, when it is
/// at least half alike.
fn pair_changed(map: &mut [usize], previous: &[&str], current: &[&str], removed: &[usize], added: &[usize]) {
    if removed.is_empty() || added.is_empty() || removed.len() * added.len() > MAX_PAIRS {
        return;
    }
    for &i in removed {
        let old = previous.get(i).map_or("", |line| line.trim());
        let mut best: Option<(f64, usize)> = None;
        for &k in added {
            let ratio = levenshtein_ratio(old, current.get(k).map_or("", |line| line.trim()));
            if best.is_none_or(|(kept, _)| ratio > kept) {
                best = Some((ratio, k));
            }
        }
        if let Some((ratio, k)) = best
            && ratio >= 0.5
        {
            set(map, i, k + 1);
        }
    }
}

fn set(map: &mut [usize], index: usize, value: usize) {
    if let Some(slot) = map.get_mut(index) {
        *slot = value;
    }
}

/// Lines `start..=end` of the previous text, as `map` sends them into a text of `count`
/// lines (crit's remapLines).
fn remap_lines(map: &[usize], [start, end]: [usize; 2], count: usize) -> [usize; 2] {
    let max = count.max(1);
    let mut s = start.checked_sub(1).and_then(|at| map.get(at)).copied().unwrap_or(0);
    let mut e = end.checked_sub(1).and_then(|at| map.get(at)).copied().unwrap_or(0);
    if s == 0 {
        s = start;
    }
    if e == 0 {
        e = end;
    }
    let s = s.min(max).max(1);
    let e = e.min(max).max(s);
    [s, e]
}

/// `long` is `short` with one run of characters added somewhere (crit's oneMiddleCut).
fn one_middle_cut(short: &str, long: &str) -> bool {
    if short.len() >= long.len() {
        return false;
    }
    short.char_indices().map(|(at, _)| at).chain(std::iter::once(short.len())).any(|k| {
        let (head, tail) = short.split_at(k);
        long.starts_with(head) && long.ends_with(tail)
    })
}

/// 1 minus the edit distance over the longer length, over characters.
fn levenshtein_ratio(a: &str, b: &str) -> f64 {
    let x: Vec<char> = a.chars().collect();
    let y: Vec<char> = b.chars().collect();
    if x.is_empty() && y.is_empty() {
        return 1.0;
    }
    let mut row: Vec<usize> = (0..=y.len()).collect();
    for (i, cx) in x.iter().enumerate() {
        let mut next = Vec::with_capacity(y.len() + 1);
        next.push(i + 1);
        for (j, cy) in y.iter().enumerate() {
            let above = row.get(j + 1).copied().unwrap_or(usize::MAX - 1) + 1;
            let left = next.last().copied().unwrap_or(usize::MAX - 1) + 1;
            let diagonal = row.get(j).copied().unwrap_or(usize::MAX - 1) + usize::from(cx != cy);
            next.push(above.min(left).min(diagonal));
        }
        row = next;
    }
    let distance = row.last().copied().unwrap_or(0);
    #[allow(clippy::cast_precision_loss, reason = "lengths of a few lines' text")]
    let ratio = 1.0 - distance as f64 / x.len().max(y.len()) as f64;
    ratio
}

/// Lines close enough to the ones a comment was made on (crit's anchorSimilar): the same once
/// trimmed; else, the shorter at least 8 bytes long, held in the longer or the longer with one
/// run cut out of it; else at least 70 % alike.
pub fn anchor_similar(candidate: &str, anchor: &str) -> bool {
    let (a, b) = (candidate.trim(), anchor.trim());
    if a == b {
        return true;
    }
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (shorter, longer) = if b.len() < a.len() { (b, a) } else { (a, b) };
    if shorter.len() >= 8 && (longer.contains(shorter) || one_middle_cut(shorter, longer)) {
        return true;
    }
    levenshtein_ratio(a, b) >= 0.7
}

/// Markers drawing strips from words: emphasis and code.
const MARKUP: [char; 4] = ['*', '_', '`', '~'];

/// `text` as quotes compare: markup markers dropped, whitespace runs as one space, lower case;
/// and each kept character's byte index in `text`.
fn flatten(text: &str) -> (Vec<char>, Vec<usize>) {
    let mut flat = Vec::new();
    let mut at = Vec::new();
    let mut space = false;
    let mut last = 0;
    for (index, ch) in text.char_indices() {
        if MARKUP.contains(&ch) {
            continue;
        }
        if ch.is_whitespace() {
            space = !flat.is_empty();
            last = index;
            continue;
        }
        if space {
            flat.push(' ');
            at.push(last);
            space = false;
        }
        for lower in ch.to_lowercase() {
            flat.push(lower);
            at.push(index);
        }
    }
    (flat, at)
}

/// Where each occurrence of `needle` starts in `flat`.
fn occurrences(flat: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > flat.len() {
        return Vec::new();
    }
    (0..=flat.len() - needle.len())
        .filter(|&from| flat.get(from..from + needle.len()) == Some(needle))
        .collect()
}

/// Whether `quote` is in `text`, both read as quotes compare.
pub fn quote_in(text: &str, quote: &str) -> bool {
    let (needle, _) = flatten(quote);
    !needle.is_empty() && !occurrences(&flatten(text).0, &needle).is_empty()
}

/// The first line of the exact run of `anchor`'s lines in `lines` nearest `near`, the earlier
/// on a tie; 0 when there is none.
fn find_anchor(lines: &[&str], anchor: &str, near: usize) -> usize {
    let count = anchor.split('\n').count();
    let mut best: usize = 0;
    for i in 0..lines.len().saturating_sub(count - 1) {
        if lines.get(i..i + count).map(|run| run.join("\n")).as_deref() != Some(anchor) {
            continue;
        }
        if best == 0 || (i + 1).abs_diff(near) < best.abs_diff(near) {
            best = i + 1;
        }
    }
    best
}

/// The lines of the occurrence of `quote` in `text` nearest byte `near`, if any.
fn find_quote(text: &str, quote: &str, near: usize) -> Option<[usize; 2]> {
    let (needle, _) = flatten(quote);
    let (flat, at) = flatten(text);
    let mut best: Option<usize> = None;
    for from in occurrences(&flat, &needle) {
        let distance = |start: usize| at.get(start).map_or(usize::MAX, |byte| byte.abs_diff(near));
        if best.is_none_or(|kept| distance(from) < distance(kept)) {
            best = Some(from);
        }
    }
    let from = best?;
    let first = *at.get(from)?;
    let last = *at.get(from + needle.len() - 1)?;
    Some([line_at(text, first), line_at(text, last)])
}

/// Where a place (`lines`, `lines_text`, `quote`) is in `current`: its lines followed from
/// `previous` (the text they were counted in, when known), checked, else looked for
/// elsewhere, else kept as followed, not found.
pub fn track_place(
    lines: [usize; 2],
    anchor: &str,
    quote: &str,
    current: &str,
    previous: Option<&str>,
) -> Tracked {
    let all = split_lines(current);
    let count = all.len();
    let followed = match previous {
        Some(before) if before != current => remap_lines(&line_map(&split_lines(before), &all), lines, count),
        _ => remap_lines(&[], lines, count),
    };
    let result = |at: [usize; 2], found: bool| Tracked {
        lines: at,
        found,
        quote_here: !quote.is_empty() && quote_in(&lines_text(current, at), quote),
    };
    let [start, end] = followed;
    if !anchor.is_empty() {
        let size = anchor.split('\n').count();
        if start + size - 1 <= count {
            let window = all.get(start - 1..start - 1 + size).map(|run| run.join("\n")).unwrap_or_default();
            if window == anchor || anchor_similar(&window, anchor) {
                return result([start, start + size - 1], true);
            }
        }
    }
    if !quote.is_empty() && quote_in(&lines_text(current, followed), quote) {
        return result(followed, true);
    }
    if !anchor.is_empty() {
        let at = find_anchor(&all, anchor, start);
        if at > 0 {
            return result([at, at + anchor.split('\n').count() - 1], true);
        }
    }
    if !quote.is_empty()
        && let Some(elsewhere) = find_quote(current, quote, line_start(current, start))
    {
        return result(elsewhere, true);
    }
    result([start, end], false)
}

/// Which text a place's lines are counted in (`hash`), of the current one and the previous one
/// the client holds.
pub fn place_base(hash: &str, current: &str, previous: Option<&str>) -> Base {
    if hash.is_empty() {
        return Base::Unknown;
    }
    if hash == text_hash(current) {
        return Base::Current;
    }
    if previous.is_some_and(|before| hash == text_hash(before)) {
        return Base::Previous;
    }
    Base::Unknown
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_retouched_in_place_keeps_its_lines() {
        let before = "# A\n\nWe ship on Friday after the review.\n\nEnd.\n";
        let after = "# A\n\nIntro.\n\nWe ship on Monday after the review.\n\nEnd.\n";
        let place =
            Place::of(before, &(before.find("We").unwrap_or(0)..before.find("review.").unwrap_or(0) + 7));
        let tracked = track_place(place.lines, &place.lines_text, "Friday", after, Some(before));
        assert_eq!(tracked, Tracked { lines: [5, 5], found: true, quote_here: false });
    }

    #[test]
    fn the_fingerprint_matches_plannotators() {
        assert_eq!(text_hash(""), "811c9dc5");
        assert_eq!(text_hash("a"), "e40c292c");
    }
}
