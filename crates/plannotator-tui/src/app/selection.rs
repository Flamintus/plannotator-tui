//! A mouse selection in document coordinates, and its conversion to a source range; the
//! finished selection waiting for its action.

use std::ops::Range;

use super::threads;

/// Anchor and head are (row, column) in document coordinates.
#[derive(Debug, Clone, Copy)]
pub(super) struct Selection {
    anchor: (usize, usize),
    head: (usize, usize),
    pub(super) dragging: bool,
}

impl Selection {
    #[cfg(test)]
    pub(crate) fn anchor(&self) -> (usize, usize) {
        self.anchor
    }

    pub(super) fn start(at: (usize, usize)) -> Self {
        Self { anchor: at, head: at, dragging: true }
    }

    pub(super) fn finished(anchor: (usize, usize), head: (usize, usize)) -> Self {
        Self { anchor, head, dragging: false }
    }

    pub(super) fn set_head(&mut self, head: (usize, usize)) {
        self.head = head;
    }

    pub(super) fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Columns of `row` covered by the selection, if any.
    pub(super) fn columns_on(&self, row: usize, row_width: usize) -> Option<Range<usize>> {
        let (a, b) = self.ordered();
        if row < a.0 || row > b.0 {
            return None;
        }
        let start = if row == a.0 { a.1 } else { 0 };
        let end = if row == b.0 { b.1 + 1 } else { row_width };
        (start < end).then_some(start..end)
    }
}

/// A finished selection waiting for an action.
#[derive(Debug, Clone)]
pub(super) struct Pending {
    pub(super) on: Selected,
    /// Document (row, col) where the selection starts; anchors the toolbar and compose box.
    pub(super) at: (usize, usize),
}

/// What a finished selection is on.
#[derive(Debug, Clone)]
pub(super) enum Selected {
    /// The document's words, as a source range.
    Source(Range<usize>),
    /// A passage of a message's words in a thread's box, which takes a sub-thread.
    Passage(threads::Passage),
}

impl Pending {
    pub(super) fn source(range: Range<usize>, at: (usize, usize)) -> Self {
        Self { on: Selected::Source(range), at }
    }

    /// The source range of a selection of the document's words.
    pub(super) fn range(&self) -> Option<&Range<usize>> {
        match &self.on {
            Selected::Source(range) => Some(range),
            Selected::Passage(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_span_rows_in_either_drag_direction() {
        let sel = Selection::finished((5, 2), (3, 5));
        assert_eq!(sel.columns_on(2, 80), None);
        assert_eq!(sel.columns_on(3, 80), Some(5..80));
        assert_eq!(sel.columns_on(4, 80), Some(0..80));
        assert_eq!(sel.columns_on(5, 80), Some(0..3));
    }
}
