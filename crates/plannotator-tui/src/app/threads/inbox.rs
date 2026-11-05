//! The flow of a document whose comments the Inbox keeps, from the Inbox's view: each
//! comment with its replies and the sub-threads on their words, the person's drafts and
//! what became of each send, the agent's replies not seen yet marked new, and a comment
//! whose words are no longer found boxed under the last block, marked so. The Inbox decides
//! what folds; a draft no one answered stays a bubble in the rail.

use std::collections::HashSet;

use plannotator_tui_inbox::{Author, ReplyNode, State as ThreadState, Status, ThreadNode};

use super::super::App;
use super::{Message, Thread};
use crate::inbox::tree;

/// What became of a send, as the flow names it.
fn status(status: Option<Status>) -> Option<String> {
    let word = match status? {
        Status::Pending => "pending",
        Status::Delivered => "delivered",
        Status::Read => "read",
        Status::Working => "working",
        Status::Replied => "replied",
        Status::Unknown => return None,
    };
    Some(word.to_owned())
}

/// What a reply's author line adds: the person's draft or send, the agent's reply new.
fn reply_tag(reply: &ReplyNode, seen: &HashSet<String>) -> Option<String> {
    match reply.author {
        Author::Person if reply.draft => Some("draft".to_owned()),
        Author::Person => status(reply.status),
        Author::Agent { .. } => (!seen.contains(&reply.id)).then(|| "new".to_owned()),
    }
}

impl Thread {
    fn of_node(node: &ThreadNode, seen: &HashSet<String>) -> Self {
        let threads = |subs: &[ThreadNode]| subs.iter().map(|sub| Self::of_node(sub, seen)).collect();
        let comment = Message {
            author: "You".to_owned(),
            tag: node.draft.then(|| "draft".to_owned()),
            body: node.text.clone(),
            threads: threads(&node.sub_threads),
        };
        let replies = node.replies.iter().map(|reply| Message {
            author: tree::author(reply),
            tag: reply_tag(reply, seen),
            body: reply.text.clone(),
            threads: threads(&reply.sub_threads),
        });
        let resolved = node.state == ThreadState::Resolved;
        // A resolved thread's last send is history: its head says resolved, and no more.
        let marks = if resolved {
            Vec::new()
        } else {
            status(node.status).into_iter().chain(node.live.then(|| "live".to_owned())).collect()
        };
        Self {
            id: node.id.clone(),
            kind: plannotator_tui_schema::Kind::Comment,
            quote: node.place.quote.clone(),
            resolved,
            folded: Some(node.folded),
            marks,
            messages: std::iter::once(comment).chain(replies).collect(),
        }
    }
}

impl App {
    /// The open document's threads by block, from the Inbox's view; `None` for a document
    /// whose notes the Inbox does not keep.
    pub(super) fn inbox_threads(&mut self) -> Option<Vec<(usize, Thread)>> {
        let (_, _, view) = self.open.store.inbox_thread()?;
        let seen = self.inbox.as_mut()?.seen(view);
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
            thread.marks.insert(0, "not found".to_owned());
            threads.push((block, thread));
        }
        Some(threads)
    }
}
