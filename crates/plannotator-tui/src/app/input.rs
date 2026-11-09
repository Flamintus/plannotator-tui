//! Keyboard and mouse handling.

use anyhow::Result;
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};

use super::compose::ComposeAction;
use super::menu::ReviewAction;
use super::selection::Selection;
use super::send::SendState;
use super::{App, Focus, GUTTER, Mode, Pending, Selected, TOOLBAR};
use crate::delivery::Delivery as _;
use crate::gate::Gate;
use crate::keys::{self, Action};

impl App {
    pub(crate) fn handle_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match &self.mode {
                Mode::Browse => self.browse_key(*key),
                Mode::ConfirmQuit => self.confirm_quit_key(*key),
                Mode::ConfirmClearAll => self.confirm_clear_all_key(*key),
                Mode::ConfirmLiveOn => {
                    self.confirm_live_mode_key(*key);
                    Ok(())
                }
                Mode::Pick => self.pick_key(*key),
                Mode::FileSearch => {
                    self.file_search_key(*key);
                    Ok(())
                }
                Mode::Archive => {
                    self.archive_key(*key);
                    Ok(())
                }
                Mode::ReviewMenu => self.menu_key(*key),
                Mode::Compose | Mode::Edit(_) | Mode::Reply(_) => self.text_key(*key),
                Mode::AttachImage(_) => self.image_prompt_key(*key),
            },
            // A paste lands in the comment box verbatim, newlines included; anywhere else
            // it is ignored rather than replayed as keystrokes.
            Event::Paste(text) if matches!(self.mode, Mode::Compose | Mode::Edit(_) | Mode::Reply(_)) => {
                self.compose.insert_text(text);
                Ok(())
            }
            // A pasted or dropped path joins the image prompt's one line.
            Event::Paste(text) if matches!(self.mode, Mode::AttachImage(_)) => {
                self.image_prompt_paste(text);
                Ok(())
            }
            // In Quick Open a paste joins the query, newlines dropped.
            Event::Paste(text) if self.mode == Mode::FileSearch => {
                self.file_search_paste(text);
                Ok(())
            }
            Event::Mouse(mouse) if self.mode == Mode::Browse => self.mouse(*mouse),
            Event::Mouse(mouse) if self.mode == Mode::Pick => self.pick_mouse(*mouse),
            Event::Mouse(mouse) if self.mode == Mode::FileSearch => {
                self.file_search_mouse(*mouse);
                Ok(())
            }
            Event::Mouse(mouse) if self.mode == Mode::ReviewMenu => self.menu_mouse(*mouse),
            Event::Mouse(mouse) if self.mode == Mode::Archive => {
                self.archive_mouse(*mouse);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn browse_key(&mut self, key: KeyEvent) -> Result<()> {
        // Global keys first.
        let action = match self.keys.action(&key, keys::GLOBAL) {
            // Quick Open needs the file tree. Without one its key means what it did before
            // `quick-open` existed: ctrl+p is `p`, which picks another reply.
            Some(Action::QuickOpen) if self.tree.is_none() => {
                let others: Vec<Action> =
                    keys::GLOBAL.iter().copied().filter(|&action| action != Action::QuickOpen).collect();
                self.keys.action(&key, &others)
            }
            action => action,
        };
        match action {
            Some(Action::QuickOpen) => return self.start_file_search(),
            Some(Action::Quit | Action::Interrupt) => {
                self.request_quit();
                return Ok(());
            }
            Some(Action::CycleFocus) => {
                self.cycle_focus();
                return Ok(());
            }
            Some(Action::Send) => return self.send_feedback(),
            Some(Action::Approve) if self.gate == Gate::Approve => return self.approve(),
            // A reply review sends every note, so one the agent already has closes like `q`.
            Some(Action::SendAndClose) if !self.is_file_review() => {
                if self.send_state == SendState::Sent {
                    self.request_quit();
                    return Ok(());
                }
                return self.send_and_quit();
            }
            Some(Action::ReviewMenu) if self.is_file_review() => {
                self.open_review_menu();
                return Ok(());
            }
            Some(Action::ResendAll) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::ResendAll);
            }
            Some(Action::FinishReview) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::Finish);
            }
            Some(Action::UndoFinish) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::Undo);
            }
            Some(Action::Archive) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::Archive);
            }
            Some(Action::ToggleTree) => {
                self.toggle_tree(self.geometry.doc.width + self.geometry.tree.width + GUTTER);
                return Ok(());
            }
            Some(Action::Reload) => return self.reload(),
            Some(Action::OpenInEditor) => {
                self.request_editor();
                return Ok(());
            }
            Some(Action::PickMessage) => {
                self.reopen_picker();
                return Ok(());
            }
            Some(Action::ToggleLiveMode) if self.review_loop.is_some() => {
                self.toggle_live_mode();
                return Ok(());
            }
            Some(Action::GeneralComment) => {
                self.general_comment();
                return Ok(());
            }
            _ => {}
        }
        match self.focus {
            Focus::Tree => self.tree_key(key),
            Focus::Document => self.document_key(key),
            Focus::Rail => self.rail_key(key),
        }
    }

    /// The quit confirmation: send first, quit anyway, or stay.
    fn confirm_quit_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.keys.action(&key, keys::CONFIRM_QUIT) {
            Some(Action::Yes | Action::Confirm) => {
                self.mode = Mode::Browse;
                self.send_and_quit()?;
            }
            Some(Action::No) => {
                self.mode = Mode::Browse;
                self.quit = true;
            }
            Some(Action::Back) => self.mode = Mode::Browse,
            _ => {}
        }
        Ok(())
    }

    /// Ask before `clear-all` removes every annotation on the open document.
    fn request_clear_all(&mut self) {
        if self.open.store.len() == 0 {
            self.status = Some("no annotations on current document".into());
        } else {
            self.mode = Mode::ConfirmClearAll;
        }
    }

    /// The clear-all confirmation: only `yes` clears; `no` or `back` keep every note.
    fn confirm_clear_all_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.keys.action(&key, keys::CONFIRM_CLEAR) {
            Some(Action::Yes) => {
                self.mode = Mode::Browse;
                let removed = self.open.store.clear_all()?;
                self.rail_cursor = 0;
                self.clear_selection();
                self.mark_unsent();
                self.sync_tree_counts();
                self.status = Some(format!("cleared {removed} annotation(s) on current document"));
            }
            Some(Action::No | Action::Back) => {
                self.mode = Mode::Browse;
                self.status = Some("clear cancelled".into());
            }
            _ => {}
        }
        Ok(())
    }

    /// The Live mode's question: only `yes` sends the drafts waiting, then turns it on.
    fn confirm_live_mode_key(&mut self, key: KeyEvent) {
        match self.keys.action(&key, keys::CONFIRM_LIVE_MODE) {
            Some(Action::Yes) => self.confirm_live_mode(),
            Some(Action::No | Action::Back) => {
                self.mode = Mode::Browse;
                self.status = Some("live mode stays off".into());
            }
            _ => {}
        }
    }

    fn cycle_focus(&mut self) {
        let has_tree = self.tree.is_some();
        let has_rail = !self.open.store.placed().is_empty();
        self.focus = match self.focus {
            Focus::Document if has_rail => Focus::Rail,
            Focus::Document | Focus::Rail if has_tree => Focus::Tree,
            Focus::Tree | Focus::Document | Focus::Rail => Focus::Document,
        };
    }

    fn tree_key(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.tree_len();
        match self.keys.action(&key, keys::TREE) {
            Some(Action::Down) => {
                self.tree_cursor = (self.tree_cursor + 1).min(len.saturating_sub(1));
            }
            Some(Action::Up) => self.tree_cursor = self.tree_cursor.saturating_sub(1),
            Some(Action::Confirm | Action::Right) => self.open_tree_selection()?,
            Some(Action::ToggleHidden) => self.toggle_tree_hidden()?,
            Some(Action::Back) => self.focus = Focus::Document,
            _ => {}
        }
        // The tree's height is the last frame's; the first frame has not drawn yet, but its cursor is row 0.
        self.keep_tree_cursor_visible(usize::from(self.geometry.tree.height));
        Ok(())
    }

    fn rail_key(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.open.store.notes().len();
        // A key the bubble decides acts only where the footer names it (`journal::help`).
        match self.keys.action(&key, keys::RAIL).filter(|&action| self.applies(action)) {
            Some(Action::Down) => {
                self.rail_cursor = (self.rail_cursor + 1).min(len.saturating_sub(1));
            }
            Some(Action::Up) => self.rail_cursor = self.rail_cursor.saturating_sub(1),
            Some(Action::Confirm | Action::Edit) => self.edit_selected_annotation(),
            Some(Action::Remove) => self.remove_selected_annotation()?,
            Some(Action::AttachImage) => self.begin_attach_image(),
            Some(Action::RemoveImage) => self.remove_image()?,
            Some(
                action @ (Action::Reply
                | Action::Resolve
                | Action::ToggleThread
                | Action::SendNow
                | Action::ToggleLive),
            ) => {
                self.thread_action(action);
            }
            Some(Action::Back) => self.focus = Focus::Document,
            _ => {}
        }
        self.follow_bubble();
        Ok(())
    }

    /// The document follows the rail's bubble: its note's block is the selected block, in view,
    /// as it always was; under the review loop its note's box is in view instead, at any depth
    /// (`show_box`), drawn open first, and every box it is in, when folded (`unfold_to`), and
    /// nothing moves while it is in view. The keys stay in the rail.
    fn follow_bubble(&mut self) {
        let notes = self.open.store.notes();
        let Some(target) = notes.get(self.rail_cursor) else { return };
        let id = target.annotation.id.clone();
        let Some(block) = self.open.doc.block_containing(target.range.start) else { return };
        self.selected = block;
        if self.box_words_selectable() && self.open.flow.unfold_to(&id) {
            self.sync_threads();
        }
        if !(self.box_words_selectable() && self.show_box(&id)) {
            self.ensure_selected_visible();
        }
    }

    fn document_key(&mut self, key: KeyEvent) -> Result<()> {
        let page = i64::from(self.geometry.doc.height.max(1));
        // With a selection waiting, the toolbar keys win.
        if self.pending.is_some() && self.keys.action(&key, keys::TOOLBAR) == Some(Action::CopySelection) {
            self.copy_pending();
            return Ok(());
        }
        if self.pending.is_some()
            && let Some(action) = self.keys.action(&key, keys::TOOLBAR)
            && let Some(&(_, _, _, kind)) = TOOLBAR.iter().find(|item| item.2 == action)
        {
            return self.act(kind);
        }
        // Visual mode: the selection follows the cursor.
        if self.selection.is_some_and(|s| s.dragging) {
            self.visual_key(key);
            return Ok(());
        }
        // Roaming (`i`): the cursor moves with nothing selected yet, so `v` can start
        // anywhere. Other keys fall through to their block-mode meaning.
        if self.roam && self.selection.is_none() {
            if self.keys.is(Action::Back, &key) {
                self.roam = false;
                self.status = None;
                return Ok(());
            }
            if self.motion_key(key) {
                return Ok(());
            }
        }
        // A key the line decides acts only where the footer names it (`journal::help`): under the
        // review loop a comment's keys act in its box, never from a line of the document.
        match self.keys.action(&key, keys::DOCUMENT).filter(|&action| self.applies(action)) {
            Some(Action::Back) => {
                if self.pending.is_some() || self.selection.is_some() {
                    self.clear_selection();
                } else {
                    self.request_quit();
                }
            }
            Some(Action::Visual) => {
                self.clear_selection();
                self.selection = Some(Selection::start(self.cursor));
                self.status = Some(format!(
                    "visual: move to extend, {} to select, {} to cancel",
                    self.keys.hint(Action::Confirm),
                    self.keys.hint(Action::Back)
                ));
            }
            Some(Action::MoveCursor) => self.start_roaming(),
            // The next unit (`unit`): a block, or a stop of the boxes on the way under the
            // review loop, a paragraph of a box's words or a thread's fold line.
            Some(Action::Down) => self.step(true),
            Some(Action::Up) => self.step(false),
            // On any line of a draft's box, its words, its head or its folded line, `edit` and
            // `confirm` edit it there, as from its bubble: a draft takes no sub-thread, and
            // `right` or a click still draws its folded line open.
            Some(Action::Edit | Action::Confirm) if self.draft_box().is_some() => {
                if let Some(id) = self.draft_box() {
                    self.edit_note(&id);
                }
            }
            // On a thread's fold line, `right` and `confirm` draw it open, `left` folds it: it
            // has no words for the cursor or a sub-thread.
            Some(action @ (Action::Left | Action::Right | Action::Confirm))
                if self.selected_fold().is_some() =>
            {
                self.fold_key(action);
            }
            // A cursor that moves must be visible, so a column move in block mode is a
            // roaming move: the same key, with the cursor drawn.
            Some(Action::Left | Action::Right) => {
                self.start_roaming();
                self.motion_key(key);
            }
            Some(Action::PageDown) => self.page_by(page / 2),
            Some(Action::PageUp) => self.page_by(-page / 2),
            Some(Action::Top) => self.select_block(0),
            Some(Action::Bottom) => {
                self.select_block(self.open.doc.blocks.len().saturating_sub(1));
            }
            // No selection: comment on the whole selected unit, the block, or a paragraph of a
            // box's words, a sub-thread on it.
            Some(Action::Comment | Action::Confirm) => self.comment_unit()?,
            // In a box, under the review loop, `clear-block` removes that box's draft, as
            // `remove` on its bubble; the footer names it only on a draft's box.
            Some(Action::ClearBlock) if self.thread_at_cursor().is_some() => {
                if let Some(id) = self.thread_at_cursor() {
                    self.remove_note(&id)?;
                }
            }
            Some(Action::ClearBlock) => {
                let removed = self.open.store.remove_in_block(&self.open.doc, self.selected)?;
                if removed > 0 {
                    self.mark_unsent();
                    self.sync_tree_counts();
                }
                self.status = Some(format!("removed {removed} annotation(s) on block"));
            }
            Some(Action::ClearAll) => self.request_clear_all(),
            Some(Action::AttachImage) => self.begin_attach_image(),
            Some(Action::RemoveImage) => self.remove_image()?,
            Some(
                action @ (Action::Reply
                | Action::Resolve
                | Action::ToggleThread
                | Action::SendNow
                | Action::ToggleLive),
            ) => {
                self.thread_action(action);
            }
            _ => {}
        }
        Ok(())
    }

    /// Keys while a keyboard selection is being extended.
    fn visual_key(&mut self, key: KeyEvent) {
        match self.keys.action(&key, keys::VISUAL) {
            Some(Action::Back) => self.clear_selection(),
            Some(Action::Confirm | Action::Visual) => self.finish_selection(),
            _ => {
                self.motion_key(key);
            }
        }
        if let Some(sel) = self.selection.as_mut() {
            sel.set_head(self.cursor);
        }
    }

    /// Enter roaming: the visual-mode motions move the cursor with nothing selected. Under the
    /// review loop the footer's right names the keys (`journal::help`), so nothing is said
    /// here; else the status names them, as it always did.
    fn start_roaming(&mut self) {
        self.clear_selection();
        self.roam = true;
        if self.open.store.is_journal() {
            self.status = None;
            return;
        }
        let hint = |action| self.keys.hint(action);
        self.status = Some(format!(
            "move: {} {} {} {} {} · {} select · {} back to blocks",
            self.keys.directions_hint(),
            hint(Action::WordForward),
            hint(Action::WordBack),
            hint(Action::LineStart),
            hint(Action::LineEnd),
            hint(Action::Visual),
            hint(Action::Back)
        ));
    }

    /// The cursor motions shared by visual and roaming modes. True when `key` was one.
    fn motion_key(&mut self, key: KeyEvent) -> bool {
        match self.keys.action(&key, keys::MOTION) {
            Some(Action::Left) => self.move_cursor(0, -1),
            Some(Action::Right) => self.move_cursor(0, 1),
            Some(Action::Down) => self.move_cursor(1, 0),
            Some(Action::Up) => self.move_cursor(-1, 0),
            Some(Action::WordForward) => self.move_word(1),
            Some(Action::WordBack) => self.move_word(-1),
            Some(Action::LineStart) => self.cursor.1 = self.row_start(self.cursor.0),
            Some(Action::LineEnd) => self.cursor.1 = self.row_width(self.cursor.0).saturating_sub(1),
            _ => return false,
        }
        self.ensure_cursor_visible();
        true
    }

    /// Move the keyboard cursor by rows/columns, skipping gap rows and clamping to text.
    fn move_cursor(&mut self, rows: i64, cols: i64) {
        let total = self.open.layout.total_rows;
        if total == 0 {
            return;
        }
        let mut row = self.cursor.0;
        for _ in 0..rows.unsigned_abs() {
            let next = if rows > 0 { row + 1 } else { row.saturating_sub(1) };
            if next >= total || next == row {
                break;
            }
            // A gap row, or a thread's box under a block: on to the next text, if there is any
            // (a box's words are text while the review loop keeps the document's comments).
            let Some(text) = self.text_row_from(next, rows > 0) else { break };
            row = text;
        }
        let (start, width) = (self.row_start(row), self.row_width(row));
        let col =
            (self.cursor.1 as i64 + cols).clamp(start as i64, width.saturating_sub(1).max(start) as i64);
        let col = col as usize;
        self.cursor = (row, col);
        if let Some(block) = self.open.layout.block_at_row(row).or_else(|| self.box_block(row)) {
            self.selected = block;
        }
        self.ensure_cursor_visible();
    }

    /// Jump to the next (+1) or previous (-1) word start on the current row.
    fn move_word(&mut self, direction: i64) {
        let Some(text) = self.row_text(self.cursor.0) else { return };
        let chars: Vec<char> = text.chars().collect();
        let is_boundary = |i: usize| {
            let here = chars.get(i).is_some_and(|c| !c.is_whitespace());
            let before = i == 0 || chars.get(i - 1).is_some_and(char::is_ascii_whitespace);
            here && before
        };
        let col = self.cursor.1;
        let next = if direction > 0 {
            (col + 1..chars.len()).find(|&i| is_boundary(i)).unwrap_or(chars.len().saturating_sub(1))
        } else {
            (0..col).rev().find(|&i| is_boundary(i)).unwrap_or(0)
        };
        self.cursor.1 = next;
    }

    fn text_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.compose.handle_key(key) {
            ComposeAction::Cancel => self.mode = Mode::Browse,
            ComposeAction::Save => {
                let body = self.compose.value().trim().to_owned();
                match std::mem::replace(&mut self.mode, Mode::Browse) {
                    Mode::Reply(id) => self.save_reply(&id, body),
                    Mode::Edit(id) => {
                        if body.is_empty() {
                            self.status = Some("edit cancelled: empty".into());
                        } else if self.open.store.edit_body(&id, body)? {
                            self.mark_unsent();
                            self.status = Some("annotation updated".into());
                        }
                    }
                    Mode::Compose
                    | Mode::AttachImage(_)
                    | Mode::Browse
                    | Mode::ConfirmQuit
                    | Mode::ConfirmClearAll
                    | Mode::ConfirmLiveOn
                    | Mode::Pick
                    | Mode::FileSearch
                    | Mode::Archive
                    | Mode::ReviewMenu => {
                        // Saved as a draft, or gone at once in the review's Live mode (its status then).
                        if !body.is_empty()
                            && let Some(pending) = self.pending.take()
                            && !self.annotate(pending.on, Kind::Comment, body)?
                        {
                            self.status = Some("comment saved".into());
                        }
                        self.clear_selection();
                    }
                }
            }
            ComposeAction::Edited => {}
        }
        Ok(())
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        match mouse.kind {
            MouseEventKind::ScrollDown if self.in_tree(mouse.column, mouse.row) => self.tree_scroll_by(3),
            MouseEventKind::ScrollUp if self.in_tree(mouse.column, mouse.row) => self.tree_scroll_by(-3),
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.send_button_hit(mouse.column, mouse.row) {
                    return self.send_feedback();
                }
                let hit = |rect: Option<ratatui::layout::Rect>| {
                    rect.is_some_and(|r| mouse.row == r.y && mouse.column >= r.x && mouse.column < r.right())
                };
                if hit(self.geometry.approve_button) {
                    return self.approve();
                }
                if hit(self.geometry.review_button) {
                    self.open_review_menu();
                    return Ok(());
                }
                if hit(self.geometry.undo_button) {
                    self.undo_finish_review();
                    return Ok(());
                }
                if let Some(kind) = self.toolbar_hit(mouse.column, mouse.row) {
                    return self.act(kind);
                }
                if let Some(index) = self.tree_hit(mouse.row, mouse.column) {
                    self.tree_cursor = index;
                    self.focus = Focus::Tree;
                    return self.open_tree_selection();
                }
                if let Some(index) = self.bubble_hit(mouse.column, mouse.row) {
                    self.rail_cursor = index;
                    self.focus = Focus::Rail;
                    // Under the review loop a bubble clicked brings its box into view, as `down`
                    // and `up` in the rail do.
                    if self.box_words_selectable() {
                        self.follow_bubble();
                    }
                    return Ok(());
                }
                let now = std::time::Instant::now();
                let double = self.last_click.take().is_some_and(|(at, col, row)| {
                    col == mouse.column
                        && row == mouse.row
                        && now.duration_since(at) <= std::time::Duration::from_millis(400)
                });
                self.last_click = Some((now, mouse.column, mouse.row));
                let Some(pos) = self.doc_position(mouse.column, mouse.row, false) else { return Ok(()) };
                self.focus = Focus::Document;
                // A click below the document's last row lands on that row (`doc_position`
                // clamps); when that row is a box's, the click is on nothing: it moves no cursor
                // and opens or folds nothing.
                let on_row =
                    self.scroll + usize::from(mouse.row.saturating_sub(self.geometry.doc.y)) == pos.0;
                if !on_row && self.box_block(pos.0).is_some() {
                    return Ok(());
                }
                // A thread's fold line, its folded line or the head of its box, has no words:
                // a click there opens or folds the thread, the cursor on the line.
                if self.fold_clicked(pos.0) {
                    return Ok(());
                }
                if double
                    && let Some(block) = self.open.layout.block_at_row(pos.0)
                    && let Some(range) = self.open.doc.blocks.get(block).map(|b| b.range.clone())
                {
                    // A double-click selects the whole block and offers the toolbar for it.
                    self.selected = block;
                    self.selection = None;
                    self.cursor = pos;
                    self.status = None;
                    self.pending = Some(Pending::source(range, pos));
                    return Ok(());
                }
                if let Some(block) = self.open.layout.block_at_row(pos.0).or_else(|| self.box_block(pos.0)) {
                    self.selected = block;
                }
                self.cursor = pos;
                self.pending = None;
                self.selection = Some(Selection::start(pos));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if !self.selection.is_some_and(|s| s.dragging) {
                    return Ok(());
                }
                let doc = self.geometry.doc;
                if mouse.row < doc.y {
                    self.scroll_by(-1);
                } else if mouse.row >= doc.bottom() {
                    self.scroll_by(1);
                }
                if let Some(pos) = self.doc_position(mouse.column, mouse.row, true)
                    && let Some(sel) = self.selection.as_mut()
                {
                    sel.set_head(pos);
                }
            }
            MouseEventKind::Up(MouseButton::Left) if self.selection.is_some_and(|s| s.dragging) => {
                self.finish_selection();
            }
            _ => {}
        }
        Ok(())
    }

    fn send_button_hit(&self, column: u16, row: u16) -> bool {
        self.geometry
            .send_button
            .is_some_and(|rect| row == rect.y && column >= rect.x && column < rect.right())
    }

    fn toolbar_hit(&self, column: u16, row: u16) -> Option<Kind> {
        let (rect, spans) = self.geometry.toolbar.as_ref()?;
        if row != rect.y || column < rect.x || column >= rect.right() {
            return None;
        }
        spans.iter().zip(TOOLBAR.iter()).find(|(span, _)| span.contains(&column)).map(|(_, item)| item.3)
    }

    /// Whether the screen cell is inside the drawn tree pane.
    fn in_tree(&self, column: u16, row: u16) -> bool {
        let tree = self.geometry.tree;
        tree.width > 0 && column >= tree.x && column < tree.right() && row >= tree.y && row < tree.bottom()
    }

    /// The tree row under the screen cell, accounting for the tree's scroll offset.
    fn tree_hit(&self, row: u16, column: u16) -> Option<usize> {
        self.in_tree(column, row)
            .then(|| self.tree_scroll + usize::from(row - self.geometry.tree.y))
            .filter(|&i| i < self.tree_len())
    }

    fn bubble_hit(&self, column: u16, row: u16) -> Option<usize> {
        let id = self
            .geometry
            .bubbles
            .iter()
            .find(|(rect, _)| {
                column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
            })
            .map(|(_, id)| id.clone())?;
        self.open.store.notes().iter().position(|p| p.annotation.id == id)
    }

    /// Screen cell -> document (row, column). With `clamp`, positions outside the
    /// document area snap to its nearest edge so a drag can leave the area.
    fn doc_position(&self, column: u16, row: u16, clamp: bool) -> Option<(usize, usize)> {
        let doc = self.geometry.doc;
        let inside = column >= doc.x && column < doc.right() && row >= doc.y && row < doc.bottom();
        if !inside && !clamp {
            let in_gutter = column >= doc.x.saturating_sub(GUTTER)
                && column < doc.x
                && row >= doc.y
                && row < doc.bottom();
            if !in_gutter {
                return None;
            }
        }
        let col = column.clamp(doc.x, doc.right().saturating_sub(1)) - doc.x;
        let row = row.clamp(doc.y, doc.bottom().saturating_sub(1)) - doc.y;
        let doc_row = (self.scroll + usize::from(row)).min(self.open.layout.total_rows.saturating_sub(1));
        Some((doc_row, usize::from(col)))
    }

    /// Turn the selection into a source range; it may span blocks. One on no text of the
    /// document may be on the words of a thread's box: a passage, which takes a sub-thread.
    pub(super) fn finish_selection(&mut self) {
        let Some(sel) = self.selection.as_mut() else { return };
        sel.dragging = false;
        if sel.is_empty() {
            self.selection = None;
            return;
        }
        let sel = *sel;
        let (a, b) = sel.ordered();
        let mut bounds: Option<(usize, usize)> = None;
        for row in a.0..=b.0 {
            let Some(r) = self.open.layout.row(row) else { continue };
            let cols = sel.columns_on(row, r.cells.len()).unwrap_or(0..0);
            for offset in r.cells.iter().skip(cols.start).take(cols.len()).flatten() {
                bounds =
                    Some(bounds.map_or((*offset, *offset), |(lo, hi)| (lo.min(*offset), hi.max(*offset))));
            }
        }
        let Some((lo, hi)) = bounds else {
            match self.box_passage(&sel) {
                Some(Ok(passage)) => self.pending_passage(passage, a),
                Some(Err(why)) => {
                    self.selection = None;
                    self.status = Some(why);
                }
                None => self.selection = None,
            }
            return;
        };
        let source = &self.open.doc.source;
        let end_char = source.get(hi..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        let range = lo..hi + end_char;
        if self.clipboard
            && self.ui.copy_on_select
            && let Some(text) = source.get(range.clone())
        {
            let _ = crate::delivery::Clipboard.deliver(text);
        }
        if let Some(block) = self.open.doc.block_containing(range.start) {
            self.selected = block;
        }
        self.status = None;
        self.pending = Some(Pending::source(range, a));
    }

    /// A passage of a box's words waits for its action, as a selection of the document does.
    fn pending_passage(&mut self, passage: super::threads::Passage, at: (usize, usize)) {
        if self.clipboard && self.ui.copy_on_select {
            let _ = crate::delivery::Clipboard.deliver(&passage.on.quote);
        }
        if let Some(block) = self.box_block(at.0) {
            self.selected = block;
        }
        self.status = None;
        self.pending = Some(Pending { on: Selected::Passage(passage), at });
    }
}
