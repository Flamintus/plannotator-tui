//! The flow of a document whose comments the review's journal keeps, from the review's view:
//! each comment with its replies and the sub-threads on their words, the person's drafts and
//! what became of each send, the agent's replies not seen yet marked new, and a comment whose
//! words are no longer found boxed at the end of the words it was on, marked so, in the words
//! the Inbox's boxes use (`Round 2`, `Not sent`, `Replied`, `Live`, `New`, `Unanchored`). One
//! code at every depth: a sub-thread is built as its thread is (`Thread::of_node`), its kind
//! and its marks with it. The view decides what folds; a draft no one answered stays a bubble
//! in the rail.

use std::collections::HashSet;
use std::ops::Range;

use plannotator_tui_journal::{Author, ReplyNode, State as ThreadState, Status, ThreadNode};
use plannotator_tui_schema::Kind;

use super::super::App;
use super::{Message, Thread};
use crate::journal::{passage, tree};
use crate::store::kind_of;

/// What became of a send, in the words the Inbox's boxes use for it.
fn status_word(status: Status) -> Option<&'static str> {
    Some(match status {
        Status::Pending => "Queued",
        Status::Delivered => "Delivered",
        Status::Read => "Read",
        Status::Working => "Working",
        Status::Replied => "Replied",
        Status::Unknown => return None,
    })
}

/// What a comment's state is, as its box's head and its bubble in the rail both say it: while
/// it is open, what became of its send and Live; then its round, or Not sent for a draft. The
/// round goes last, so a narrow head gives it up first.
pub(in crate::app) fn state_marks(node: &ThreadNode) -> Vec<String> {
    let mut marks = Vec::new();
    if node.state != ThreadState::Resolved {
        marks.extend(node.status.and_then(status_word).map(str::to_owned));
        marks.extend(node.live.then(|| "Live".to_owned()));
    }
    marks.push(if node.draft { "Not sent".to_owned() } else { format!("Round {}", node.round) });
    marks
}

/// The kind a box's head names, when it is not a comment: its glyph alone may be missed.
fn kind_word(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Comment => None,
        Kind::LooksGood => Some("Looks good"),
        Kind::Delete => Some("Deletion"),
    }
}

/// What a reply says beside its author: its status or New, never its round or Not sent, which
/// the box's head says already.
fn reply_tag(reply: &ReplyNode, seen: &HashSet<String>) -> Option<String> {
    let after = match reply.author {
        Author::Person if reply.draft => None,
        Author::Person => reply.status.and_then(status_word),
        Author::Agent { .. } => (!seen.contains(&reply.id)).then_some("New"),
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
    /// says (`Unanchored`); its sub-threads are built the same way on their messages' words.
    fn of_node(node: &ThreadNode, seen: &HashSet<String>, range: Option<Range<usize>>) -> Self {
        let threads = |subs: &[ThreadNode], words: &str| {
            subs.iter().map(|sub| Self::of_node(sub, seen, passage_range(words, sub))).collect()
        };
        let comment = Message {
            id: node.id.clone(),
            author: "You".to_owned(),
            // Its round, or Not sent, is in the box's head already: the author stands alone.
            tag: None,
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
        let kind = kind_of(node);
        let mut marks: Vec<String> = kind_word(kind).map(str::to_owned).into_iter().collect();
        marks.extend(range.is_none().then(|| "Unanchored".to_owned()));
        // A resolved thread's last send is history: its head says resolved and its round.
        marks.extend(state_marks(node));
        Self {
            id: node.id.clone(),
            kind,
            quote: node.place.quote.clone(),
            range,
            resolved: node.state == ThreadState::Resolved,
            folded: Some(node.folded),
            marks,
            messages: std::iter::once(comment).chain(replies).collect(),
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
        let mut threads = Vec::new();
        for placed in open.store.placed() {
            let (Some(block), Some(node)) =
                (open.doc.block_containing(placed.range.start), node(&placed.annotation.id))
            else {
                continue;
            };
            if node.draft && node.replies.is_empty() && node.sub_threads.is_empty() {
                continue;
            }
            threads.push((block, Thread::of_node(node, seen, Some(placed.range.clone()))));
        }
        let last = open.doc.blocks.len().checked_sub(1);
        for orphan in open.store.orphaned() {
            let (Some(block), Some(node)) = (last, node(&orphan.id)) else { continue };
            threads.push((block, Thread::of_node(node, seen, None)));
        }
        Some(threads)
    }
}
