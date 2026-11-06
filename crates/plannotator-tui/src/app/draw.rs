//! Drawing: header, optional tree, gutter + document, annotation rail, footer, and the
//! floating toolbar and compose box. Pure over `App` except for recording geometry for
//! hit-testing.

use plannotator_tui_schema::{Kind, State};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{App, Focus, GUTTER, Geometry, Mode, Selected, TOOLBAR, bubble, glyph, label, threads};
use crate::gate::Gate;
use crate::keys::Action;
use crate::theme::palette;

const RAIL_WIDTH: u16 = 36;
const RAIL_MIN_WIDTH: u16 = 28;
/// Below this the rail is dropped and annotations are only marked in the gutter.
const RAIL_MIN_TOTAL_WIDTH: u16 = 80;
const TREE_WIDTH: u16 = 28;
/// Below this the tree is hidden unless toggled on; Tab still reaches it.
pub(super) const TREE_MIN_TOTAL_WIDTH: u16 = 120;
const COMPOSE_WIDTH: u16 = 48;

pub(super) fn accent(kind: Kind) -> Color {
    match kind {
        Kind::Comment => palette().comment,
        Kind::LooksGood => palette().looks_good,
        Kind::Delete => palette().delete,
    }
}

/// The border of a note lit (under the review loop, `threads::lit_notes`): its bubble's in the
/// rail, and under the review loop its box's in the flow (`threads`): green once its thread is
/// resolved, else its kind's colour.
pub(super) fn lit(kind: Kind, resolved: bool) -> Style {
    Style::new().fg(if resolved { palette().looks_good } else { accent(kind) })
}

/// Painting precedence when annotations overlap a cell.
pub(super) fn priority(kind: Kind) -> u8 {
    match kind {
        Kind::Comment => 0,
        Kind::LooksGood => 1,
        Kind::Delete => 2,
    }
}

/// How a passage an annotation of `kind` is on is drawn: in the document's words, and in a
/// box's words for a sub-thread's.
pub(super) fn marked(kind: Kind) -> Style {
    match kind {
        Kind::Comment => Style::new().bg(palette().comment_bg),
        Kind::LooksGood => Style::new().bg(palette().approve_bg),
        Kind::Delete => Style::new().fg(palette().delete).add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
    }
}

impl App {
    pub(crate) fn draw(&mut self, frame: &mut Frame) {
        let area = self.viewport(frame.area());
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(self.header_height(area.width)),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let show_tree = self.tree_shown(area.width) || (self.tree.is_some() && self.focus == Focus::Tree);
        let tree_width = if show_tree { TREE_WIDTH } else { 0 };
        let rail_width = if area.width.saturating_sub(tree_width) >= RAIL_MIN_TOTAL_WIDTH {
            (area.width * 3 / 10).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH)
        } else {
            0
        };
        // `[ui] doc_width` caps the text; the columns it leaves sit between it and the rail.
        // Only a cap that binds changes the split, so an unset or roomy cap lays out as before.
        let gap = u16::from(rail_width > 0);
        let room = body.width.saturating_sub(tree_width + GUTTER + gap + rail_width);
        let cap = self.ui.doc_width.max(20);
        let (text, spare) = if self.ui.doc_width > 0 && room > cap {
            (Constraint::Length(cap), Constraint::Min(gap))
        } else {
            (Constraint::Min(20), Constraint::Length(gap))
        };
        let [tree, gutter, doc, _gap, rail] = Layout::horizontal([
            Constraint::Length(tree_width),
            Constraint::Length(GUTTER),
            text,
            spare,
            Constraint::Length(rail_width),
        ])
        .areas(body);
        self.geometry = Geometry { tree, doc, ..Geometry::default() };

        if self.open.layout.width != usize::from(doc.width) {
            self.open.layout.reflow(usize::from(doc.width));
            self.clear_selection();
            self.scroll_by(0);
        }
        self.sync_threads();
        self.keep_field_in_view();

        self.draw_header(frame, header);
        if show_tree {
            // A resize may have changed the tree's height; keep the window in range.
            self.tree_scroll_by(0);
            self.draw_tree(frame, tree);
        }
        self.draw_document(frame, gutter, doc);
        self.draw_threads(frame.buffer_mut(), gutter, doc);
        if rail_width > 0 {
            self.draw_rail(frame, rail);
        }
        self.draw_footer(frame, footer);
        match &self.mode {
            Mode::Compose => self.draw_typed(frame, "comment"),
            Mode::Edit(_) => self.draw_typed(frame, "edit"),
            Mode::Reply(_) => self.draw_typed(frame, "reply"),
            Mode::AttachImage(_) => {
                self.draw_compose(frame, " image path \u{b7} enter attaches \u{b7} esc cancels ");
            }
            Mode::Browse if self.pending.is_some() => self.draw_toolbar(frame),
            Mode::Pick => self.draw_pick(frame),
            Mode::FileSearch => self.draw_file_search(frame),
            Mode::Archive => self.draw_archive(frame),
            Mode::ReviewMenu => self.draw_review_menu(frame),
            Mode::Browse | Mode::ConfirmQuit | Mode::ConfirmClearAll => {}
        }
    }

    fn draw_tree(&self, frame: &mut Frame, area: Rect) {
        let Some(tree) = &self.tree else { return };
        let focused = self.focus == Focus::Tree;
        let border =
            if focused { Style::new().fg(palette().accent) } else { Style::new().fg(Color::DarkGray) };
        let block = Block::default().borders(Borders::RIGHT).border_style(border);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let open_path = match &self.open.source.provenance {
            plannotator_tui_schema::Provenance::File { path } => Some(path.as_path()),
            _ => None,
        };
        let lines: Vec<Line<'static>> = tree
            .rows
            .iter()
            .enumerate()
            .skip(self.tree_scroll)
            .take(usize::from(inner.height))
            .map(|(i, row)| {
                let indent = "  ".repeat(row.depth);
                let name = if row.is_dir {
                    let arrow = if row.expanded { "\u{25be}" } else { "\u{25b8}" };
                    format!("{indent}{arrow} {}/", row.name)
                } else {
                    format!("{indent}{}", row.name)
                };
                let mut style = if row.is_dir { Style::new().dim() } else { Style::new() };
                if open_path == Some(row.path.as_path()) {
                    style = style.bold().fg(palette().accent);
                }
                let row_bg = (focused && i == self.tree_cursor).then(|| palette().block_bg);
                if let Some(bg) = row_bg {
                    style = style.bg(bg);
                }
                // Name on the left, annotation count right-aligned; nothing shown at zero.
                let count = if row.annotations > 0 { row.annotations.to_string() } else { String::new() };
                let width = usize::from(inner.width);
                let name: String = name.chars().take(width.saturating_sub(count.len() + 2)).collect();
                let pad = width.saturating_sub(1 + name.chars().count() + count.len());
                let with_bg = |s: Style| row_bg.map_or(s, |bg| s.bg(bg));
                Line::from(vec![
                    Span::styled(format!(" {name}"), style),
                    Span::styled(" ".repeat(pad), with_bg(Style::new())),
                    Span::styled(count, with_bg(Style::new().fg(palette().comment))),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_document(&self, frame: &mut Frame, gutter: Rect, doc: Rect) {
        let placed = self.open.store.placed();
        let text_selection_active = self.selection.is_some();
        let doc_focused = self.focus == Focus::Document;
        // A line of a box's words selected instead of the block above it (`box_line_selected`).
        let selected = (!self.box_line_selected()).then_some(self.selected);
        let buf = frame.buffer_mut();

        for y in 0..doc.height {
            let row_index = self.scroll + usize::from(y);
            let Some(block) = self.open.layout.block_at_row(row_index) else { continue };
            let Some(row) = self.open.layout.row(row_index) else { continue };
            let screen_y = doc.y + y;
            buf.set_line(doc.x, screen_y, &row.line, doc.width);

            if Some(block) == selected && !text_selection_active && self.pending.is_none() && doc_focused {
                buf.set_style(
                    Rect { x: doc.x, y: screen_y, width: doc.width, height: 1 },
                    Style::new().bg(palette().block_bg),
                );
            }

            let mut row_has_annotation = false;
            for (col, cell) in row.cells.iter().enumerate().take(usize::from(doc.width)) {
                let Some(offset) = cell else { continue };
                let kind = placed
                    .iter()
                    .filter(|p| p.range.contains(offset))
                    .map(crate::store::Placed::kind)
                    .max_by_key(|&k| priority(k));
                let Some(kind) = kind else { continue };
                row_has_annotation = true;
                buf.set_style(Rect { x: doc.x + col as u16, y: screen_y, width: 1, height: 1 }, marked(kind));
            }

            if let Some(cols) = self.selection.and_then(|s| s.columns_on(row_index, row.cells.len().max(1))) {
                let start = cols.start.min(usize::from(doc.width)) as u16;
                let end = cols.end.min(usize::from(doc.width)) as u16;
                if end > start {
                    let rect = Rect { x: doc.x + start, y: screen_y, width: end - start, height: 1 };
                    buf.set_style(rect, palette().selection);
                }
            }

            // Keyboard cursor, visible while roaming or selecting with the keyboard.
            if doc_focused
                && (self.roam || self.selection.is_some_and(|s| s.dragging))
                && row_index == self.cursor.0
            {
                let x = doc.x + (self.cursor.1.min(usize::from(doc.width).saturating_sub(1))) as u16;
                buf.set_style(Rect { x, y: screen_y, width: 1, height: 1 }, palette().cursor);
            }

            let marker = match (Some(block) == selected, row_has_annotation) {
                (true, _) => Span::styled("▍", Style::new().fg(palette().accent)),
                (false, true) => Span::styled("▍", Style::new().fg(palette().comment)),
                (false, false) => Span::raw(" "),
            };
            buf.set_span(gutter.x, screen_y, &marker, 1);
        }
    }

    /// Screen position for a floating widget anchored at the pending selection: one row
    /// above its first row when there is room, else just below its last row.
    fn float_origin(&self, height: u16, width: u16) -> Option<Rect> {
        let pending = self.pending.as_ref()?;
        let doc = self.geometry.doc;
        let (row, col) = pending.at;
        if row < self.scroll || row >= self.scroll + usize::from(doc.height) {
            return None;
        }
        let screen_row = doc.y + (row - self.scroll) as u16;
        let width = width.min(doc.width);
        let x = (doc.x + col as u16).min(doc.right().saturating_sub(width));
        let y = if screen_row >= doc.y + height {
            screen_row - height
        } else {
            let last_row = self.selection.map_or(row, |s| s.ordered().1.0);
            let below = doc.y + last_row.saturating_sub(self.scroll) as u16 + 1;
            below.min(doc.bottom().saturating_sub(height))
        };
        Some(Rect { x, y, width, height })
    }

    fn draw_toolbar(&mut self, frame: &mut Frame) {
        let labels: Vec<String> = TOOLBAR
            .iter()
            .map(|(g, l, a, _)| match self.keys.hint(*a) {
                key if key.is_empty() => format!(" {g} {l} "),
                key => format!(" {g} {l} ({key}) "),
            })
            .collect();
        let width: u16 = labels.iter().map(|l| l.width() as u16).sum::<u16>() + 1;
        let Some(rect) = self.float_origin(1, width) else { return };
        frame.render_widget(Clear, rect);
        let buf = frame.buffer_mut();
        buf.set_style(rect, Style::new().bg(palette().toolbar_bg));
        let mut x = rect.x + 1;
        let mut spans = [0..0, 0..0, 0..0];
        for ((label, item), span) in labels.iter().zip(TOOLBAR.iter()).zip(spans.iter_mut()) {
            let w = label.width() as u16;
            let style = Style::new().fg(accent(item.3)).bg(palette().toolbar_bg).bold();
            buf.set_span(x, rect.y, &Span::styled(label.as_str(), style), w);
            *span = x..x + w;
            x += w;
        }
        self.geometry.toolbar = Some((rect, spans));
    }

    /// The compose box: at the pending selection when there is one, else over the rail
    /// bubble being edited, else centered.
    /// The compose box title; the Shift+Enter hint appears only when the terminal
    /// actually distinguishes it, so the hint is never a lie.
    pub(in crate::app) fn compose_title(&self, verb: &str) -> String {
        let newline = if self.shift_enter { "shift+enter new line" } else { "alt+enter new line" };
        format!(" {verb} \u{b7} enter saves \u{b7} {newline} \u{b7} esc cancels ")
    }

    /// The compose box, `verb` in its title: in the box the person types about, when there is
    /// one (`threads::field`), else floating.
    fn draw_typed(&self, frame: &mut Frame, verb: &str) {
        match self.typed_field() {
            Some(field) => self.draw_field(frame, field),
            None => self.draw_compose(frame, &self.compose_title(verb)),
        }
    }

    fn draw_compose(&self, frame: &mut Frame, title: &str) {
        let wrap_width = usize::from(COMPOSE_WIDTH.saturating_sub(3));
        let content_rows = self.compose.rows(wrap_width);
        let height = content_rows as u16 + 2;
        let rect = self
            .float_origin(height, COMPOSE_WIDTH)
            .or_else(|| self.edit_origin(height, COMPOSE_WIDTH))
            .unwrap_or_else(|| {
                let area = self.viewport(frame.area());
                let width = COMPOSE_WIDTH.min(area.width);
                Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height / 2, width, height }
            });
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().comment))
            .title(Span::styled(title.to_owned(), Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let words = Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(1),
            height: content_rows as u16,
            ..inner
        };
        self.compose_words(frame, words, wrap_width);
    }

    /// What the person types over the rows of its field, in the box it is about
    /// (`threads::field`), `field` as `typed_field` gives it: the rows of it in view, the words
    /// wrapped to the field's.
    fn draw_field(&self, frame: &mut Frame, (first, rows, left): (usize, usize, usize)) {
        let doc = self.geometry.doc;
        let view = self.scroll..self.scroll + usize::from(doc.height);
        let shown = first.max(view.start)..(first + rows).min(view.end);
        if shown.is_empty() {
            return;
        }
        let width = usize::from(doc.width).saturating_sub(2 * left);
        let words = Rect {
            x: doc.x + left as u16,
            y: doc.y + (shown.start - self.scroll) as u16,
            width: width as u16,
            height: shown.len() as u16,
        };
        self.compose_words(frame, words, width);
    }

    /// The comment's words wrapped to `wrap` columns, in `area`'s rows: when it is taller, the
    /// rows past them scroll so the cursor's row is in view; the cursor on it.
    fn compose_words(&self, frame: &mut Frame, area: Rect, wrap: usize) {
        let (lines, cursor_row, cursor_col) = self.compose.wrapped(wrap);
        let rows = usize::from(area.height).max(1);
        let scroll = cursor_row.saturating_sub(rows - 1);
        for (i, line) in lines.iter().skip(scroll).take(rows).enumerate() {
            let row = Rect { y: area.y + i as u16, height: 1, ..area };
            frame.render_widget(Paragraph::new(Line::from(line.clone())), row);
        }
        let cursor_x = area.x + cursor_col as u16;
        let cursor_y = area.y + (cursor_row - scroll) as u16;
        frame.set_cursor_position((cursor_x.min(area.right().saturating_sub(1)), cursor_y));
    }

    fn edit_origin(&self, height: u16, width: u16) -> Option<Rect> {
        let (Mode::Edit(id) | Mode::AttachImage(id) | Mode::Reply(id)) = &self.mode else { return None };
        let (rect, _) = self.geometry.bubbles.iter().find(|(_, bubble_id)| bubble_id == id)?;
        let area = self.geometry.doc.union(*rect);
        let x = rect.right().saturating_sub(width).max(area.x);
        Some(Rect { x, y: rect.y, width: width.min(area.width), height })
    }

    /// The notes' bubbles. Under the review loop they stack from the top, in the document's
    /// order, a sub-thread's after its thread's note, as the column of Plannotator's screens
    /// lists its comments, and the ones lit are the boxes lit (`lit_notes`): the current note's,
    /// which the keys act on, in the rail or in a box at any depth, every note of the selected
    /// block on a line of the document; without it each sits level with its passage, as it
    /// always did, and every note of the selected block is lit.
    fn draw_rail(&mut self, frame: &mut Frame, rail: Rect) {
        let view_end = self.scroll + usize::from(rail.height);
        let rail_focused = self.focus == Focus::Rail;
        // Under the review loop the replies are under the passage, in a box or a folded line.
        let in_loop = self.open.store.is_journal();
        let inner_width = usize::from(rail.width.saturating_sub(4));
        let mut next_y = rail.y;
        let notes = self.open.store.notes();
        // The bubbles lit are the boxes lit (`lit_notes`): in a box, a sub-thread's alone, though
        // its note is on its thread's block.
        let lit_notes = if in_loop { self.lit_notes() } else { Vec::new() };
        let first_lit = notes.iter().position(|p| lit_notes.contains(&p.annotation.id));
        let first = if in_loop { first_stacked(&notes, first_lit, inner_width, rail.height) } else { 0 };
        let mut bubbles = Vec::new();
        for (index, placed) in notes.iter().enumerate().skip(first) {
            let Some(block) = self.open.doc.block_containing(placed.range.start) else { continue };
            let y = if in_loop {
                next_y
            } else {
                let Some(rendered) = self.open.layout.blocks.get(block) else { continue };
                let anchor_row =
                    self.open.layout.first_row_in_range(block, placed.range).unwrap_or(rendered.first_row);
                if anchor_row + 1 < self.scroll.saturating_sub(2)
                    || anchor_row >= view_end
                    || next_y >= rail.bottom()
                {
                    continue;
                }
                let anchored_y = rail.y + anchor_row.saturating_sub(self.scroll) as u16;
                anchored_y.max(next_y)
            };
            let kind = placed.kind();
            let lines = bubble::lines(placed.annotation, label(kind), inner_width, in_loop);
            let height = (lines.len() as u16 + 2).min(rail.bottom().saturating_sub(y));
            if height < 3 {
                break;
            }
            let highlighted = if in_loop {
                lit_notes.contains(&placed.annotation.id)
            } else if rail_focused {
                index == self.rail_cursor
            } else {
                block == self.selected
            };
            let resolved = in_loop && placed.annotation.state == State::Resolved;
            // Lit as the box of the same note is (`threads`); a resolved thread is only seen
            // as such under the review loop.
            let border = if highlighted { lit(kind, resolved) } else { Style::new().fg(Color::DarkGray) };
            let border = if rail_focused && index == self.rail_cursor { border.bold() } else { border };
            // A comment of the journal's says what became of its send instead (`journal_tag`).
            let sent = if self.is_file_review()
                && !self.open.store.is_journal()
                && !self.open.store.is_pending(placed.annotation)
            {
                " · sent"
            } else {
                ""
            };
            let images = match placed.annotation.plannotator_tui.images.len() {
                0 => String::new(),
                1 => " · 1 image".to_owned(),
                n => format!(" · {n} images"),
            };
            let journal = self.journal_tag(&placed.annotation.id);
            let title_style = Style::new().fg(accent(kind));
            let mut title = vec![Span::styled(
                format!(" {} {}{images}{sent}", glyph(kind), short_id(&placed.annotation.id)),
                title_style,
            )];
            if resolved {
                title.push(Span::styled(" \u{b7} ", title_style));
                title.push(Span::styled(threads::RESOLVED, threads::resolved_style()));
            }
            title.push(Span::styled(format!("{journal} "), title_style));
            let title = Line::from(title);
            let bubble = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border)
                .title(title);
            let rect = Rect { x: rail.x, y, width: rail.width, height };
            let inner = bubble.inner(rect);
            frame.render_widget(bubble, rect);
            let text_area = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
            frame.render_widget(Paragraph::new(lines), text_area);
            bubbles.push((rect, placed.annotation.id.clone()));
            next_y = y + height;
        }
        self.geometry.bubbles = bubbles;
    }

    fn draw_footer(&mut self, frame: &mut Frame, mut area: Rect) {
        if self.mode == Mode::ConfirmQuit {
            // The question owns the footer: the browse help would name keys that are not
            // live while it is up.
            let answers = [(Action::Yes, "send"), (Action::No, "quit"), (Action::Back, "cancel")]
                .into_iter()
                .filter_map(|(action, label)| self.keys.labelled(action, label))
                .collect::<Vec<_>>()
                .join(" · ");
            let question =
                format!(" send feedback to {} before quitting? {answers}", self.delivery.describe());
            frame.render_widget(Paragraph::new(Line::from(Span::raw(question).bold())), area);
            return;
        }
        if self.mode == Mode::ConfirmClearAll {
            let answers = [(Action::Yes, "clear"), (Action::No, "cancel")]
                .into_iter()
                .filter_map(|(action, label)| self.keys.labelled(action, label))
                .collect::<Vec<_>>()
                .join(" · ");
            let question =
                format!(" clear all {} annotation(s) on current document? {answers}", self.open.store.len());
            frame.render_widget(Paragraph::new(Line::from(Span::raw(question).bold())), area);
            return;
        }
        if self.mode == Mode::Browse && !self.undo_archive.is_empty() {
            let label = self.keys.labelled(Action::UndoFinish, "Undo finish");
            let label = format!(" {} ", label.as_deref().unwrap_or("Undo finish"));
            let width = (label.width() as u16).min(area.width);
            let rect = Rect { x: area.right() - width, y: area.y, width, height: area.height };
            frame
                .render_widget(Paragraph::new(label.as_str()).style(Style::new().fg(palette().accent)), rect);
            self.geometry.undo_button = Some(rect);
            area.width = area.width.saturating_sub(width);
        }
        let orphans = self.open.store.orphans();
        // The status leads: it is the transient half of the line, and the name and counters
        // it pushes right are on screen for the whole session anyway.
        let mut parts: Vec<String> = self.status.iter().cloned().collect();
        parts.extend([
            self.open.source.name.clone(),
            format!(
                "{} annotations{}",
                self.open.store.len(),
                if orphans > 0 { format!(" ({orphans} orphaned)") } else { String::new() }
            ),
            match &self.pending {
                Some(p) => {
                    let chars = match &p.on {
                        Selected::Source(range) => {
                            self.open.doc.source.get(range.clone()).map_or(0, |s| s.chars().count())
                        }
                        Selected::Passage(passage) => passage.on.quote.chars().count(),
                    };
                    format!("selected {chars} chars")
                }
                None => format!("block {}/{}", self.selected + 1, self.open.doc.blocks.len()),
            },
        ]);
        if self.viewport(frame.area()).width < RAIL_MIN_TOTAL_WIDTH {
            parts.push("rail hidden: widen to ≥80 cols".into());
        }
        let help = self.footer_help();
        // The status must stay readable at any width, so the key help yields columns to it
        // (and is clipped) rather than the other way round.
        let status_width = self.status.as_ref().map_or(0, |s| s.width() + 1) as u16;
        let help_width = (help.width() as u16).min(area.width.saturating_sub(status_width.max(10)));
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Min(10), Constraint::Length(help_width)]).areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(format!(" {}", parts.join(" · "))).dim())),
            left_area,
        );
        frame.render_widget(Paragraph::new(Line::from(Span::raw(help).dim()).right_aligned()), right_area);
    }

    /// The key help, in the user's keys, a space after it: under the review loop what the
    /// selection lets the keys do, then, past a wider `·`, the keys that do not change with it
    /// (`journal::help`); else the focused pane's, as it always was.
    fn footer_help(&self) -> String {
        let groups = self.loop_help().unwrap_or_else(|| [self.pane_help(), Vec::new()]);
        let groups: Vec<String> =
            groups.iter().filter(|group| !group.is_empty()).map(|group| group.join(" \u{b7} ")).collect();
        if groups.is_empty() { String::new() } else { format!("{} ", groups.join("  \u{b7}  ")) }
    }

    /// `down` and `up`'s first keys run together, as the hints show them: `j/k`; none when
    /// both are unbound.
    pub(super) fn up_down_hint(&self) -> Option<String> {
        let keys: Vec<String> = [Action::Down, Action::Up]
            .map(|action| self.keys.hint(action))
            .into_iter()
            .filter(|k| !k.is_empty())
            .collect();
        (!keys.is_empty()).then(|| keys.join("/"))
    }

    /// The key help for the focused pane, without the review loop's journal.
    fn pane_help(&self) -> Vec<String> {
        // Each hint names its action's first key; an unbound action drops out of the line.
        let key = |action, label| self.keys.labelled(action, label);
        let bare = |hint: String| (!hint.is_empty()).then_some(hint);
        let up_down = self.up_down_hint();
        let tab = bare(self.keys.hint(Action::CycleFocus));
        // `send-and-close` is bound only in a reply review, so only a reply review names it.
        let send_and_close =
            if self.is_file_review() { None } else { key(Action::SendAndClose, "send+quit") };
        let quit = [send_and_close, key(Action::Quit, "quit")];
        let approve = (self.gate == Gate::Approve).then(|| key(Action::Approve, "approve")).flatten();
        // The image keys act on the current note; removing is offered only when it has one.
        let note = self.current_note();
        let has_images = note.as_deref().is_some_and(|id| self.open.store.has_images(id));
        let remove_image = has_images.then(|| key(Action::RemoveImage, "remove image")).flatten();
        let help: Vec<Option<String>> = match self.focus {
            _ if self.pending.is_some() => vec![
                key(Action::LooksGood, "looks good"),
                key(Action::Comment, "comment"),
                key(Action::Delete, "delete"),
                (!self.ui.copy_on_select).then(|| key(Action::CopySelection, "copy")).flatten(),
                key(Action::Back, "clear"),
            ],
            Focus::Tree => vec![
                up_down,
                key(Action::Confirm, "open"),
                key(Action::ToggleHidden, "hidden"),
                key(Action::Send, "send"),
                approve,
                key(Action::ToggleTree, "hide"),
                key(Action::Quit, "quit"),
            ],
            Focus::Rail => [
                up_down,
                key(Action::Edit, "edit"),
                key(Action::Remove, "remove"),
                key(Action::AttachImage, "image"),
                remove_image,
                tab,
            ]
            .into_iter()
            .chain(quit)
            .collect(),
            Focus::Document if self.roam => [
                bare(self.keys.directions_hint()).map(|keys| format!("{keys} move")),
                key(Action::Visual, "select"),
                key(Action::Comment, "comment"),
                key(Action::Back, "blocks"),
            ]
            .into_iter()
            .chain(quit)
            .collect(),
            Focus::Document => [
                key(Action::MoveCursor, "move"),
                key(Action::Visual, "select"),
                key(Action::Comment, "comment"),
                note.is_some().then(|| key(Action::AttachImage, "image")).flatten(),
                remove_image,
                key(Action::Send, "send"),
                approve,
                tab,
            ]
            .into_iter()
            .chain(quit)
            .collect(),
        };
        // Quick Open works from any pane of a folder review, so every pane's help names it,
        // last.
        let find =
            if self.tree.is_some() && self.pending.is_none() { key(Action::QuickOpen, "find") } else { None };
        help.into_iter().chain([find]).flatten().collect()
    }
}

/// The first of the stacked bubbles drawn: the top one, unless the first of those lit
/// (`first_lit`, by its place in `notes`) would be cut; then the rail scrolls only as far as
/// shows it whole.
fn first_stacked(
    notes: &[crate::store::Placed<'_>],
    first_lit: Option<usize>,
    width: usize,
    height: u16,
) -> usize {
    let tall =
        |p: &crate::store::Placed<'_>| bubble::lines(p.annotation, label(p.kind()), width, true).len() + 2;
    let mut first = first_lit.unwrap_or(0).min(notes.len().saturating_sub(1));
    let mut used = notes.get(first).map_or(0, tall);
    while let Some(above) = first.checked_sub(1).and_then(|index| notes.get(index)) {
        used += tall(above);
        if used > usize::from(height) {
            break;
        }
        first -= 1;
    }
    first
}

/// The tail of an id, enough to tell bubbles apart: `anno_…F0123` → `F0123`.
pub(super) fn short_id(id: &str) -> String {
    let tail: Vec<char> = id.chars().rev().take(5).collect();
    tail.into_iter().rev().collect()
}

/// A picker's bottom help: the arrows that move its cursor, then the bound keys.
pub(super) fn picker_help(moves: &str, keys: impl IntoIterator<Item = Option<String>>) -> String {
    let parts: Vec<String> =
        std::iter::once(format!("\u{2191}\u{2193} {moves}")).chain(keys.into_iter().flatten()).collect();
    format!(" {} ", parts.join(" \u{b7} "))
}
