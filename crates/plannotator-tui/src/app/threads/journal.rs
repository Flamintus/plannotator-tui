//! The flow of a document whose comments the review's journal keeps, from the review's view:
//! each comment with its replies and the sub-threads on their words, the person's drafts and
//! what became of each send, the agent's replies not seen yet marked new, and a comment whose
//! words are no longer found boxed under the last block, marked so, in the words the Inbox's
//! boxes use (`Round 2`, `Not sent`, `Replied`, `Live`, `New`, `Unanchored`). The view decides
//! what folds; a draft no one answered stays a bubble in the rail.

use std::collections::HashSet;

use plannotator_tui_journal::{Author, ReplyNode, State as ThreadState, Status, ThreadNode};

use super::super::App;
use super::{Message, Thread};
use crate::journal::{passage, tree};

/// What became of a send, in the words the Inbox's boxes use for it.
pub(in crate::app) fn status_word(status: Status) -> Option<&'static str> {
    Some(match status {
        Status::Pending => "Queued",
        Status::Delivered => "Delivered",
        Status::Read => "Read",
        Status::Working => "Working",
        Status::Replied => "Replied",
        Status::Unknown => return None,
    })
}

/// The round a message went or came in, or that the person's draft is not sent yet.
fn round(draft: bool, round: u64) -> String {
    if draft { "Not sent".to_owned() } else { format!("Round {round}") }
}

/// What a reply's author line adds: its round, then what became of the person's send, or the
/// agent's reply new.
fn reply_tag(reply: &ReplyNode, seen: &HashSet<String>) -> String {
    let after = match reply.author {
        Author::Person if reply.draft => None,
        Author::Person => reply.status.and_then(status_word),
        Author::Agent { .. } => (!seen.contains(&reply.id)).then_some("New"),
    };
    let round = round(reply.draft, reply.round);
    match after {
        Some(after) => format!("{round} \u{b7} {after}"),
        None => round,
    }
}

/// Where `sub`'s passage starts in `words`, the message it is on: the byte of the occurrence
/// nearest the journal's offset, as Plannotator finds it; `None` when the words do not hold it.
fn passage_start(words: &str, sub: &ThreadNode) -> Option<usize> {
    let chars: Vec<(char, usize)> = words.char_indices().map(|(at, ch)| (ch, at)).collect();
    let quote = &sub.place.quote;
    let found = match &sub.place.parent {
        Some(parent) => passage::find_near(&chars, words, quote, parent.quote_offset),
        None => passage::find(&chars, words, quote, 0),
    };
    found.map(|range| range.start)
}

impl Thread {
    fn of_node(node: &ThreadNode, seen: &HashSet<String>) -> Self {
        let threads = |subs: &[ThreadNode], words: &str| {
            let sub =
                |sub: &ThreadNode| Self { start: passage_start(words, sub), ..Self::of_node(sub, seen) };
            subs.iter().map(sub).collect()
        };
        let comment = Message {
            id: node.id.clone(),
            author: "You".to_owned(),
            tag: Some(round(node.draft, node.round)),
            body: node.text.clone(),
            draft: node.draft,
            threads: threads(&node.sub_threads, &node.text),
        };
        let replies = node.replies.iter().map(|reply| Message {
            id: reply.id.clone(),
            author: tree::author(reply),
            tag: Some(reply_tag(reply, seen)),
            body: reply.text.clone(),
            draft: reply.draft,
            threads: threads(&reply.sub_threads, &reply.text),
        });
        let resolved = node.state == ThreadState::Resolved;
        // A resolved thread's last send is history: its head says resolved and its round. The
        // round goes last, so a narrow head gives it up first.
        let mut marks = Vec::new();
        if !resolved {
            marks.extend(node.status.and_then(status_word).map(str::to_owned));
            marks.extend(node.live.then(|| "Live".to_owned()));
        }
        marks.push(round(node.draft, node.round));
        Self {
            id: node.id.clone(),
            kind: plannotator_tui_schema::Kind::Comment,
            quote: node.place.quote.clone(),
            start: None,
            resolved,
            folded: Some(node.folded),
            marks,
            messages: std::iter::once(comment).chain(replies).collect(),
        }
    }
}

impl App {
    /// The open document's threads by block, from the review's view; `None` for a document
    /// whose notes the journal does not keep.
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
            let mut thread = Thread::of_node(node, seen);
            thread.kind = placed.kind();
            threads.push((block, thread));
        }
        let last = open.doc.blocks.len().checked_sub(1);
        for orphan in open.store.orphaned() {
            let (Some(block), Some(node)) = (last, node(&orphan.id)) else { continue };
            let mut thread = Thread::of_node(node, seen);
            thread.marks.insert(0, "Unanchored".to_owned());
            threads.push((block, thread));
        }
        Some(threads)
    }
}
