//! Per-block rendering and the row map.
//!
//! Each block is rendered in isolation through `tui-markdown` (cached, width-independent),
//! aligned back to its source bytes, and wrapped to the current column width. Every screen
//! cell therefore knows its block and, where it shows real text, its source byte.

use std::ops::Range;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use tui_markdown::{Options, StyleSheet};

use crate::doc::{BlockKind, Document};
use crate::srcmap::{LineOffsets, align};
use crate::theme::palette;
use crate::wrap::{Row, clip_line, wrap_line, wrap_table};

/// Rows of vertical space between blocks.
const BLOCK_GAP: usize = 1;

/// House style: no `#` markers, headings carry weight through bold/underline rather than
/// background color, so the palette stays available for selection and annotations.
#[derive(Debug, Clone)]
struct Styles;

impl StyleSheet for Styles {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().fg(palette().heading).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::new().fg(palette().heading).add_modifier(Modifier::BOLD),
            _ => Style::new().fg(palette().subheading).add_modifier(Modifier::BOLD),
        }
    }
    fn heading_marker(&self, _level: u8) -> &'static str {
        ""
    }
    fn code(&self) -> Style {
        Style::new().fg(Color::LightYellow)
    }
    fn link(&self) -> Style {
        Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED)
    }
    fn blockquote(&self) -> Style {
        Style::new().fg(Color::Green).add_modifier(Modifier::ITALIC)
    }
}

#[derive(Debug)]
pub(crate) struct RenderedBlock {
    /// Width-independent styled lines from the renderer (owned, cached).
    text: Text<'static>,
    /// Per line, per char: absolute source byte offset (cached with `text`).
    offsets: Vec<LineOffsets>,
    kind: BlockKind,
    /// Rows for the current width.
    pub(crate) rows: Vec<Row>,
    /// First screen row of this block in document coordinates.
    pub(crate) first_row: usize,
}

#[derive(Debug)]
pub(crate) struct DocLayout {
    pub(crate) width: usize,
    pub(crate) blocks: Vec<RenderedBlock>,
    pub(crate) total_rows: usize,
    /// Rows kept free right under a block, as `(block, rows)` in block order: where the app
    /// draws the threads on that block's passages. Like a gap, they belong to no block.
    under: Vec<(usize, usize)>,
    /// Rows kept free above the first block: where the app draws the comments on the whole
    /// document. They belong to no block either.
    top: usize,
}

fn render_block(doc: &Document, index: usize) -> (Text<'static>, Vec<LineOffsets>) {
    let source = doc.block_text(index);
    let text = own(tui_markdown::from_str_with_options(source, &Options::new(Styles)));
    let plain: Vec<String> = text.lines.iter().map(ToString::to_string).collect();
    let base = doc.blocks.get(index).map_or(0, |b| b.range.start);
    let offsets = align(&plain, source, base);
    (text, offsets)
}

fn own(text: Text<'_>) -> Text<'static> {
    let lines = text
        .lines
        .into_iter()
        .map(|line| {
            let spans =
                line.spans.into_iter().map(|s| ratatui::text::Span::styled(s.content.into_owned(), s.style));
            Line::from(spans.collect::<Vec<_>>()).style(line.style)
        })
        .collect::<Vec<_>>();
    Text::from(lines).style(text.style)
}

impl DocLayout {
    /// Render every block once (the expensive part) and lay out for `width`.
    pub(crate) fn build(doc: &Document, width: usize) -> Self {
        let blocks = doc
            .blocks
            .iter()
            .enumerate()
            .map(|(i, block)| {
                let (text, offsets) = render_block(doc, i);
                RenderedBlock { text, offsets, kind: block.kind, rows: Vec::new(), first_row: 0 }
            })
            .collect();
        let mut layout = Self { width: 0, blocks, total_rows: 0, under: Vec::new(), top: 0 };
        layout.reflow(width);
        layout
    }

    /// Re-wrap for a new width without re-rendering markdown.
    pub(crate) fn reflow(&mut self, width: usize) {
        let width = width.max(1);
        self.width = width;
        for block in &mut self.blocks {
            let lines = || block.text.lines.iter().zip(&block.offsets);
            block.rows = if block.kind == BlockKind::Table {
                wrap_table(&block.text.lines, &block.offsets, width)
            } else if block.kind.preserves_columns() {
                lines().map(|(l, o)| clip_line(l, o, width)).collect()
            } else {
                lines().flat_map(|(l, o)| wrap_line(l, o, width)).collect()
            };
        }
        self.place();
    }

    /// Keep `under` rows free under the blocks it names (`(block, rows)`, in block order) and
    /// move the blocks below down by as much, without re-wrapping. Returns whether it moved
    /// anything.
    pub(crate) fn reserve_under(&mut self, under: &[(usize, usize)]) -> bool {
        if self.under == under {
            return false;
        }
        under.clone_into(&mut self.under);
        self.place();
        true
    }

    /// Keep `rows` free above the first block and move every block down by as much, without
    /// re-wrapping. Returns whether it moved anything.
    pub(crate) fn reserve_top(&mut self, rows: usize) -> bool {
        if self.top == rows {
            return false;
        }
        self.top = rows;
        self.place();
        true
    }

    /// Each block's first row, from the rows kept above the first block, the wrapped rows, the
    /// rows kept under blocks and the gaps.
    fn place(&mut self) {
        let mut under = self.under.iter().peekable();
        let mut row = self.top;
        for (index, block) in self.blocks.iter_mut().enumerate() {
            block.first_row = row;
            row += block.rows.len() + BLOCK_GAP;
            if let Some((_, rows)) = under.next_if(|(at, _)| *at == index) {
                row += rows;
            }
        }
        self.total_rows = row.saturating_sub(BLOCK_GAP);
    }

    /// Which block owns a document row (gap rows belong to nobody).
    pub(crate) fn block_at_row(&self, row: usize) -> Option<usize> {
        let idx = self.blocks.partition_point(|b| b.first_row <= row);
        let i = idx.checked_sub(1)?;
        let block = self.blocks.get(i)?;
        (row < block.first_row + block.rows.len()).then_some(i)
    }

    /// The nearest row from `row` on, going down (or up), that shows text: past a gap and
    /// past the rows kept under a block. `None` when there is none that way.
    pub(crate) fn text_row_from(&self, row: usize, down: bool) -> Option<usize> {
        if self.row(row).is_some() {
            return Some(row);
        }
        let after = self.blocks.partition_point(|b| b.first_row <= row);
        if down {
            self.blocks.iter().skip(after).find(|b| !b.rows.is_empty()).map(|b| b.first_row)
        } else {
            let before = self.blocks.iter().take(after).rev().find(|b| !b.rows.is_empty());
            before.map(|b| b.first_row + b.rows.len() - 1)
        }
    }

    /// The row at document coordinate `row`, if it exists (None for gap rows).
    pub(crate) fn row(&self, row: usize) -> Option<&Row> {
        let block = self.blocks.get(self.block_at_row(row)?)?;
        block.rows.get(row - block.first_row)
    }

    /// First document row on which any cell falls inside `range`.
    pub(crate) fn first_row_in_range(&self, block: usize, range: &Range<usize>) -> Option<usize> {
        let b = self.blocks.get(block)?;
        b.rows
            .iter()
            .position(|r| r.cells.iter().any(|c| c.is_some_and(|o| range.contains(&o))))
            .map(|i| b.first_row + i)
    }

    /// The rendered text under a source range, in the web client's form: rendered
    /// characters whose source offset falls in `range`, blocks joined with no separator.
    ///
    /// The renderer maps a soft line break to nothing, so a break inside a wrapping block
    /// is recovered from the source: when consecutive rendered characters skip over
    /// whitespace in the source, one space is emitted (the DOM renders a soft break as a
    /// space). Code and tables keep their newlines.
    pub(crate) fn rendered_in_range(&self, source: &str, range: &Range<usize>) -> String {
        self.rendered_chars(source, range).into_iter().map(|(ch, _)| ch).collect()
    }

    /// `rendered_in_range`'s characters, each with the source offset it shows; a break
    /// recovered from the source carries the offset of the character after it.
    pub(crate) fn rendered_chars(&self, source: &str, range: &Range<usize>) -> Vec<(char, usize)> {
        let mut out: Vec<(char, usize)> = Vec::new();
        for block in &self.blocks {
            let break_char = if block.kind.preserves_columns() { '\n' } else { ' ' };
            let mut last_offset: Option<usize> = None;
            let chars = block.text.lines.iter().zip(&block.offsets).flat_map(|(line, offsets)| {
                line.spans.iter().flat_map(|s| s.content.chars()).zip(offsets.iter())
            });
            for (ch, offset) in chars {
                let Some(offset) = offset.filter(|o| range.contains(o)) else { continue };
                if let Some(prev) = last_offset
                    && let Some(skipped) = source.get(prev..offset)
                    && skipped.chars().any(char::is_whitespace)
                    && !out.last().is_some_and(|(last, _)| last.is_whitespace())
                {
                    out.push((break_char, offset));
                }
                out.push((ch, offset));
                last_offset = Some(offset + ch.len_utf8());
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn rendered_text_strips_markup_and_joins_blocks_without_separator() {
        let doc = Document::parse("Ship the **login page**\nby Friday.\n\nNext para.\n".to_owned());
        let layout = DocLayout::build(&doc, 80);
        let whole = 0..doc.source.len();
        assert_eq!(layout.rendered_in_range(&doc.source, &whole), "Ship the login page by Friday.Next para.");
        let bold = doc.source.find("**login").expect("present");
        let bold_range = bold..bold + "**login page**".len();
        assert_eq!(layout.rendered_in_range(&doc.source, &bold_range), "login page");
    }

    fn kept_under_the_first(rows: usize) -> DocLayout {
        let doc = Document::parse("First.\n\nSecond.\n".to_owned());
        let mut layout = DocLayout::build(&doc, 80);
        layout.reserve_under(&[(0, rows)]);
        layout
    }

    #[test]
    fn rows_kept_under_a_block_push_the_next_one_down_and_belong_to_no_block() {
        let mut layout = kept_under_the_first(3);
        for _ in 0..2 {
            assert_eq!(layout.blocks.get(1).map(|b| b.first_row), Some(5), "3 rows, then the gap");
            assert_eq!(layout.total_rows, 6);
            assert!((1..5).all(|row| layout.block_at_row(row).is_none()), "the kept rows are nobody's");
            layout.reflow(40);
        }
    }

    #[test]
    fn the_nearest_text_row_steps_over_the_rows_kept_under_a_block() {
        let layout = kept_under_the_first(3);
        assert_eq!(layout.text_row_from(1, true), Some(5));
        assert_eq!(layout.text_row_from(3, false), Some(0));
        assert_eq!(layout.text_row_from(5, true), Some(5), "a text row is its own nearest");
    }

    #[test]
    fn every_cell_of_a_long_table_maps_to_its_source() {
        let doc = Document::parse(
            "| Item | Status | Next step |\n|---|---|---|\n\
| A. Four changes from 3/7 (release checklist handoff, the 3/4 planning call, the 3/6 search demo, \
the 3/4-3/7 progress review and rollout advice) | Scope partly confirmed; never added to the \
draft | 1.0d design + 1.0d build + 1.0d review + 2.5d rollout = **5.5d** |\n\
| B. 3/14 cache troubleshooting and the `api.example.com` proxy | Owner and priority not set | Your call |\n\
| C. 3/19-20 export access in the admin panel (read and write) | Agreed at 1.0d, done once it's \
tested; it's live, but QA hasn't finished acceptance testing | Close it now, or wait for the next sprint |\n\
| D. 3/21 lookup and report tools in the CLI; 3/23 plugin consolidation | Marked \"not confirmed\" | Your call |\n"
                .to_owned(),
        );
        let layout = DocLayout::build(&doc, 100);
        let table = layout.blocks.iter().find(|b| b.kind == BlockKind::Table).expect("a table");
        let mut cells = 0;
        for row in &table.rows {
            let text: String = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
            for (ch, offset) in text.chars().zip(&row.cells) {
                if ch.is_alphanumeric() {
                    let source = offset.and_then(|o| doc.source.get(o..));
                    assert!(source.is_some_and(|s| s.starts_with(ch)), "{ch:?} is unmapped in {text:?}");
                    cells += 1;
                }
            }
        }
        assert!(cells > 300, "only {cells} cells checked");
    }
}
