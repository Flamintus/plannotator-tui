//! The person's reply typed right in the box it answers, at its bottom under its last message,
//! as Plannotator's boxes take it, at every depth and of every kind. While `reply` is typed
//! (`Mode::Reply`), the box of the comment it answers is drawn open, and so is every box holding
//! it, with the reply's field at its bottom: a box in the box, the compose box's title at its
//! head, its words as many rows as the reply takes, up to the compose box's (`compose::ROWS`),
//! past which they scroll as there. The field takes its rows in the flow as a message does, so
//! what is under it moves down, and the view follows it; its words and the cursor are drawn over
//! its rows each frame, as the selection is over the boxes' words. Folding by hand is kept: once
//! the reply is saved or cancelled, each box is drawn as it was.

use super::super::{App, Mode};
use super::Thread;
use super::rows::inside;

/// The field a reply is typed in: the compose box's title, and the rows its words take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Field {
    pub(super) title: String,
    pub(super) rows: usize,
}

impl Thread {
    /// Make room for the person's reply in the box of thread `id`, this one or one under it at
    /// any depth, this one's box `width` columns wide: at the bottom of that box, the field
    /// `field` gives for the columns of its words; that box, and every box holding it, drawn
    /// open while the reply is typed. Whether `id` is this thread or one under it.
    pub(super) fn replying(&mut self, id: &str, width: usize, field: &dyn Fn(usize) -> Field) -> bool {
        let inner = inside(width);
        let here = self.id == id;
        let holds = here
            || self
                .messages
                .iter_mut()
                .flat_map(|message| &mut message.threads)
                .any(|sub| sub.replying(id, inner, field));
        if here {
            // The field is a box in the box: its words are inside it.
            self.reply = Some(field(inside(inner)));
        }
        if holds {
            self.by_hand = Some(false);
        }
        holds
    }
}

impl App {
    /// The words of the field the person's reply is typed in, while it is: the document row of
    /// their first row, how many rows they take, and the columns left of them, as many as right
    /// of them.
    pub(in crate::app) fn reply_field(&self) -> Option<(usize, usize, usize)> {
        if !matches!(self.mode, Mode::Reply(_)) {
            return None;
        }
        let layout = &self.open.layout;
        self.open.flow.rows.iter().find_map(|(block, rows)| {
            let rendered = layout.blocks.get(*block)?;
            let top = rendered.first_row + rendered.rows.len();
            let first = rows.iter().position(|row| row.field)?;
            let count = rows.iter().skip(first).take_while(|row| row.field).count();
            Some((top + first, count, rows.get(first)?.words.len()))
        })
    }

    /// While the reply is typed, the view follows its field: the whole of it in view, from its
    /// title to its bottom edge, its title first when the view is shorter.
    pub(in crate::app) fn keep_reply_in_view(&mut self) {
        let Some((first, rows, _)) = self.reply_field() else { return };
        let height = usize::from(self.geometry.doc.height.max(1));
        let (title, bottom) = (first.saturating_sub(1), first + rows);
        if bottom >= self.scroll + height {
            self.scroll = bottom + 1 - height;
        }
        if title < self.scroll {
            self.scroll = title;
        }
    }
}
