//! The flow of a document whose comments the review's journal keeps, from the review's view:
//! each comment with its replies and the sub-threads on their words, the person's drafts and
//! what became of each send, the agent's replies not seen yet marked new, and a comment whose
//! words are no longer found boxed at the end of the words it was on, marked so, in the words
//! the Inbox's boxes use (`Orphaned`, `⚡ Live`, `Round 2`, `Draft`, `Sent`, `Replied`, `New`). One
//! code at every depth: a sub-thread is built as its thread is (`Thread::of_node`), its kind
//! and its marks with it, a box in the state of the person's last message in it (`box_state`),
//! each message of the person's, its comment included, saying beside "You" the state it is
//! still in (`persons_tag`). The view decides what folds.

use std::collections::HashSet;
use std::ops::Range;

use plannotator_tui_journal::{Author, ReplyNode, State as ThreadState, Status, ThreadNode};

use super::super::App;
use super::head::{DRAFT, Mark, NEW, QUEUED, READ, REPLIED, RESOLVED, SENT, WORKING};
use super::{Message, Thread};
use crate::journal::{passage, tree};
use crate::store::kind_of;

/// What became of a send, in the words the Inbox's boxes use for it.
fn status_word(status: Status) -> Option<&'static str> {
    Some(match status {
        Status::Pending => QUEUED,
        Status::Delivered => SENT,
        Status::Read => READ,
        Status::Working => WORKING,
        Status::Replied => REPLIED,
        Status::Unknown => return None,
    })
}

/// What a comment's head says of it, its box's, its folded line's and its bubble's alike
/// (`head::Mark`, which places them): `Orphaned` once its words are no longer found
/// (`anchored` false); `⚡ Live` while it is open; its comment's round, but for a draft, which
/// has none; and its box's state (`box_state`). A resolved thread says neither what became of
/// its last send nor Live.
fn marks(node: &ThreadNode, anchored: bool) -> Vec<Mark> {
    let resolved = node.state == ThreadState::Resolved;
    [
        (!anchored).then_some(Mark::Orphaned),
        (node.live && !resolved).then_some(Mark::Live),
        (!node.draft).then_some(Mark::Round(node.round)),
        box_state(node).map(Mark::State),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The state of `node`'s box, the one its head, its folded line and its bubble say and its
/// border takes lit: `Resolved` once the thread is; else the state of the person's last message
/// in it, the comment or a reply of theirs under it, `Draft` until it is sent, then what became
/// of its send, `Replied` once the agent answered after it. A box answered where the person
/// wrote again is in the state of what they wrote, not of its comment.
fn box_state(node: &ThreadNode) -> Option<&'static str> {
    if node.state == ThreadState::Resolved {
        return Some(RESOLVED);
    }
    let last = node.replies.iter().rev().find(|reply| matches!(reply.author, Author::Person));
    let (draft, status) = last.map_or((node.draft, node.status), |reply| (reply.draft, reply.status));
    if draft { Some(DRAFT) } else { status.and_then(status_word) }
}

/// What a message of the person's says beside "You", its comment or a reply, at any depth: its
/// states that are not final only, `Draft` until it is sent (a message is written a draft), then
/// `Sent` until the agent answers (`Queued`, `Read` and `Working` say so too), then nothing.
/// Never its round, which the box's head says.
fn persons_tag(draft: bool, status: Option<Status>) -> Option<&'static str> {
    if draft {
        return Some(DRAFT);
    }
    match status? {
        Status::Replied | Status::Unknown => None,
        Status::Pending | Status::Delivered | Status::Read | Status::Working => Some(SENT),
    }
}

/// What a reply says beside its author: the person's, what each of their messages says
/// (`persons_tag`); an agent's `New` until the person sees it.
fn reply_tag(reply: &ReplyNode, seen: &HashSet<String>) -> Option<String> {
    let after = match reply.author {
        Author::Person => persons_tag(reply.draft, reply.status),
        Author::Agent { .. } => (!seen.contains(&reply.id)).then_some(NEW),
    };
    after.map(str::to_owned)
}

/// Where `sub`'s passage is in `words`, the message it is on: the bytes of the occurrence
/// nearest the journal's offset, as Plannotator finds it; `None` when the words do not hold it.
fn passage_range(words: &str, sub: &ThreadNode) -> Option<Range<usize>> {
    let chars: Vec<(char, usize)> = words.char_indices().map(|(at, ch)| (ch, at)).collect();
    let quote = &sub.place.quote;
    match &sub.place.parent {
        Some(parent) => passage::find_near(&chars, words, quote, parent.quote_offset),
        None => passage::find(&chars, words, quote, 0),
    }
}

impl Thread {
    /// `node`'s thread, at any depth: `range` is where its passage is in the words it is on
    /// (the document's, or its message's), `None` when they no longer hold it, which its head
    /// says (`Orphaned`); `images` counts the images of a note by its id. Its sub-threads are
    /// built the same way on their messages' words.
    fn of_node(
        node: &ThreadNode,
        seen: &HashSet<String>,
        range: Option<Range<usize>>,
        images: &impl Fn(&str) -> usize,
    ) -> Self {
        let threads = |subs: &[ThreadNode], words: &str| {
            subs.iter().map(|sub| Self::of_node(sub, seen, passage_range(words, sub), images)).collect()
        };
        let comment = Message {
            id: node.id.clone(),
            author: "You".to_owned(),
            // The first message is the last too while nothing answers it: it says its state.
            tag: persons_tag(node.draft, node.status).map(str::to_owned),
            body: node.text.clone(),
            draft: node.draft,
            threads: threads(&node.sub_threads, &node.text),
        };
        let replies = node.replies.iter().map(|reply| Message {
            id: reply.id.clone(),
            author: tree::author(reply),
            tag: reply_tag(reply, seen),
            body: reply.text.clone(),
            draft: reply.draft,
            threads: threads(&reply.sub_threads, &reply.text),
        });
        Self {
            id: node.id.clone(),
            kind: kind_of(node),
            quote: node.place.quote.clone(),
            marks: marks(node, range.is_some()),
            range,
            resolved: node.state == ThreadState::Resolved,
            folded: Some(node.folded),
            by_hand: None,
            images: images(&node.id),
            messages: std::iter::once(comment).chain(replies).collect(),
            field: None,
            top: false,
        }
    }
}

impl App {
    /// The open document's threads by block, from the review's view; `None` for a document
    /// whose notes the journal does not keep. A comment whose words are no longer found is
    /// under the last block, as a sub-thread's is at the end of its message's words.
    pub(super) fn journal_threads(&mut self) -> Option<Vec<(usize, Thread)>> {
        let (_, view) = self.open.store.journal_view()?;
        let seen = self.review_loop.as_mut()?.seen(view);
        let open = &self.open;
        let node = |id: &str| tree::find(&view.threads, id);
        let images = |id: &str| open.store.images(id);
        let mut threads = Vec::new();
        for placed in open.store.placed() {
            let (Some(block), Some(node)) =
                (open.doc.block_containing(placed.range.start), node(&placed.annotation.id))
            else {
                continue;
            };
            threads.push((block, Thread::of_node(node, seen, Some(placed.range.clone()), &images)));
        }
        // A comment whose quote is not in its place is under the block its place starts in (the
        // next one when it starts between two), its place marked, Orphaned when its words were
        // not found (`plannotator_tui_schema::place`).
        for (place, found) in open.store.kept_places() {
            let start = place.range.start;
            let block = open.doc.block_containing(start).or_else(|| {
                open.doc.blocks.iter().position(|block| block.range.start >= start).or(open
                    .doc
                    .blocks
                    .len()
                    .checked_sub(1))
            });
            let (Some(block), Some(node)) = (block, node(&place.annotation.id)) else { continue };
            let mut thread = Thread::of_node(node, seen, Some(place.range.clone()), &images);
            if !found {
                thread.marks.insert(0, Mark::Orphaned);
            }
            threads.push((block, thread));
        }
        let last = open.doc.blocks.len().checked_sub(1);
        for orphan in open.store.orphaned() {
            let (Some(block), Some(node)) = (last, node(&orphan.id)) else { continue };
            threads.push((block, Thread::of_node(node, seen, None, &images)));
        }
        // A comment on the whole document is above the first block, as the web's is above the
        // title: never Orphaned, it has no words to lose.
        for general in open.store.general() {
            let Some(node) = node(&general.annotation.id) else { continue };
            let mut thread = Thread::of_node(node, seen, None, &images);
            thread.marks.retain(|mark| *mark != Mark::Orphaned);
            thread.top = true;
            threads.push((0, thread));
        }
        // In the document's order, as the flow places them: the ones above the first block,
        // then under their blocks.
        threads.sort_by_key(|(block, thread)| {
            (!thread.top, *block, thread.range.as_ref().map_or(usize::MAX, |range| range.start))
        });
        Some(threads)
    }
}
