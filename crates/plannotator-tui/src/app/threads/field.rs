//! What the person types about a box is typed right in it, as Plannotator's boxes take it, at
//! every depth and of every kind, by one code; only the place differs (`Place`). A reply
//! (`Mode::Reply`) is at the bottom of the box it answers, under its last message; a comment on a
//! passage of a message's words (`Mode::Compose` on a passage: a sub-thread) is under the
//! paragraph the passage starts in, where the sub-thread's box will be, among the boxes there in
//! their passages' order; the edit of a note (`Mode::Edit`), a draft's, is in its box in place of
//! its words. While it is typed, that box is drawn open, and so is every box holding it, with
//! the field in it: a box in the box, the compose box's title at its head, its words as many rows
//! as they take, up to the compose box's (`compose::ROWS`), past which they scroll as there, a
//! blank row above and below it. The field takes its rows in the flow as a message does, so what
//! is under it moves down, and the view follows it; its words and the cursor are drawn over its
//! rows each frame, as the selection is over the boxes' words. Folding by hand is kept: once it
//! is saved or cancelled, each box is drawn as it was. A box too narrow to draw (its folded line
//! instead) takes no field: the compose box floats, as it does without the loop.

use super::super::{App, Mode, Selected};
use super::Thread;
use super::rows::inside;

/// Where a field is in its box: what is typed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// A reply: at the bottom of the box, under its last message.
    Bottom,
    /// An edit of the box's note: in place of the words of its first message.
    Words,
    /// A comment on a passage of the words of message `message` (its place in the box), from
    /// their byte `start`: under the paragraph it starts in, where the sub-thread's box will be.
    Passage { message: usize, start: usize },
}

/// The field typed in a box: where, the compose box's titles (what is typed, on its top edge;
/// its keys, on its bottom edge), and the rows its words take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Field {
    pub(super) place: Place,
    pub(super) title: String,
    pub(super) keys: String,
    pub(super) rows: usize,
}

/// The box a field goes in, found at any depth.
#[derive(Debug, Clone)]
pub(super) enum Target {
    /// Thread `id`'s box, at `place` (`Bottom`, `Words`).
    Thread(String, Place),
    /// The box holding message `id`, a passage of its words from byte `start` (`Place::Passage`).
    Message(String, usize),
}

impl Thread {
    /// Make room for the field in the box `target` names, this one or one under it at any depth,
    /// this one's box `width` columns wide: there, at its place, the field `field` gives for it
    /// and the columns of its words; that box, and every box holding it, drawn open while it is
    /// typed. Whether that box is this one or one under it.
    pub(super) fn typing(
        &mut self,
        target: &Target,
        width: usize,
        field: &dyn Fn(Place, usize) -> Field,
    ) -> bool {
        let inner = inside(width);
        let place = match target {
            Target::Thread(id, place) => (self.id == *id).then_some(*place),
            Target::Message(id, start) => {
                let message = self.messages.iter().position(|message| message.id == *id);
                message.map(|message| Place::Passage { message, start: *start })
            }
        };
        let holds = place.is_some()
            || self
                .messages
                .iter_mut()
                .flat_map(|message| &mut message.threads)
                .any(|sub| sub.typing(target, inner, field));
        if let Some(place) = place {
            // The field is a box in the box: its words are inside it.
            self.field = Some(field(place, inside(inner)));
        }
        if holds {
            self.by_hand = Some(false);
        }
        holds
    }
}

impl App {
    /// The box the person types in, while they do: a reply's, an edit's, a comment's on a
    /// passage of a box's words. A comment on the document's words is typed in no box.
    pub(super) fn typing(&self) -> Option<Target> {
        match &self.mode {
            Mode::Reply(id) => Some(Target::Thread(id.clone(), Place::Bottom)),
            Mode::Edit(id) => Some(Target::Thread(id.clone(), Place::Words)),
            Mode::Compose => match &self.pending.as_ref()?.on {
                Selected::Passage(passage) => Some(Target::Message(passage.on.parent.clone(), passage.start)),
                Selected::Source(_) => None,
            },
            _ => None,
        }
    }

    /// The words of the field the person types in, in a box, while they do: the document row of
    /// their first row, how many rows they take, and the columns left of them, as many as right
    /// of them.
    pub(in crate::app) fn typed_field(&self) -> Option<(usize, usize, usize)> {
        if !matches!(self.mode, Mode::Reply(_) | Mode::Edit(_) | Mode::Compose) {
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

    /// While the person types in a box, the view follows the field: the whole of it in view,
    /// from its title to its bottom edge, its title first when the view is shorter.
    pub(in crate::app) fn keep_field_in_view(&mut self) {
        let Some((first, rows, _)) = self.typed_field() else { return };
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
