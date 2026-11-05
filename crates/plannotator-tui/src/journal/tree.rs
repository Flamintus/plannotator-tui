//! Walking the view of a review's threads, a tree: each comment with its replies, and the
//! sub-threads on the words of each, at any depth. Read only: Plannotator computes the tree;
//! this finds what is in it.

use plannotator_tui_journal::{Author, ReplyNode, Status, ThreadNode, ThreadView};

/// Comment or sub-thread `id`, at any depth.
pub(crate) fn find<'a>(nodes: &'a [ThreadNode], id: &str) -> Option<&'a ThreadNode> {
    nodes.iter().find_map(|node| {
        if node.id == id {
            return Some(node);
        }
        find(&node.sub_threads, id)
            .or_else(|| node.replies.iter().find_map(|reply| find(&reply.sub_threads, id)))
    })
}

/// The thread whose comment or reply is message `id`, at any depth.
pub(crate) fn holding<'a>(nodes: &'a [ThreadNode], id: &str) -> Option<&'a ThreadNode> {
    nodes.iter().find_map(|node| {
        if node.id == id || node.replies.iter().any(|reply| reply.id == id) {
            return Some(node);
        }
        holding(&node.sub_threads, id)
            .or_else(|| node.replies.iter().find_map(|reply| holding(&reply.sub_threads, id)))
    })
}

/// The sub-threads under `node`, at any depth, in the order its box draws them: on the
/// comment's words, then on each reply's, each message's by where their passages start, each
/// followed by the ones under it.
pub(crate) fn under(node: &ThreadNode) -> Vec<&ThreadNode> {
    let mut out = Vec::new();
    let messages =
        std::iter::once(&node.sub_threads).chain(node.replies.iter().map(|reply| &reply.sub_threads));
    for subs in messages {
        let mut subs: Vec<&ThreadNode> = subs.iter().collect();
        subs.sort_by_key(|sub| sub.place.parent.as_ref().map(|parent| parent.quote_offset));
        for sub in subs {
            out.push(sub);
            out.extend(under(sub));
        }
    }
    out
}

/// The drafts of `node` and of everything under it: what one note's send carries.
pub(crate) fn drafts(node: &ThreadNode, out: &mut Vec<String>) {
    if node.draft {
        out.push(node.id.clone());
    }
    for reply in &node.replies {
        if reply.draft {
            out.push(reply.id.clone());
        }
        for sub in &reply.sub_threads {
            drafts(sub, out);
        }
    }
    for sub in &node.sub_threads {
        drafts(sub, out);
    }
}

/// The drafts a send at once of `node` carries, as a box's Send does: the comment itself when it
/// is a draft, else the person's replies waiting under it. The sub-threads on its words are
/// threads of their own, sent from their boxes.
pub(crate) fn now_drafts(node: &ThreadNode) -> Vec<String> {
    if node.draft {
        return vec![node.id.clone()];
    }
    node.replies.iter().filter(|reply| reply.draft).map(|reply| reply.id.clone()).collect()
}

/// Every draft of the review, in the view's order.
pub(crate) fn all_drafts(view: &ThreadView) -> Vec<String> {
    let mut out = Vec::new();
    for node in &view.threads {
        drafts(node, &mut out);
    }
    out
}

/// The person's comments, replies and sub-threads that send `send` carried, at any depth.
pub(crate) fn carried_by(view: &ThreadView, send: &str) -> Vec<String> {
    fn walk(node: &ThreadNode, send: &str, out: &mut Vec<String>) {
        if node.reply_id == send {
            out.push(node.id.clone());
        }
        for reply in &node.replies {
            if matches!(reply.author, Author::Person) && reply.message_id == send {
                out.push(reply.id.clone());
            }
            for sub in &reply.sub_threads {
                walk(sub, send, out);
            }
        }
        for sub in &node.sub_threads {
            walk(sub, send, out);
        }
    }
    let mut out = Vec::new();
    for node in &view.threads {
        walk(node, send, &mut out);
    }
    out
}

/// The sends nothing marked delivered, read or answered, by id, newest last: what carried a
/// thread or a reply of the person's whose status is pending.
pub(crate) fn pending_sends(view: &ThreadView) -> Vec<String> {
    fn walk(node: &ThreadNode, out: &mut Vec<String>) {
        if node.status == Some(Status::Pending) && !node.reply_id.is_empty() {
            out.push(node.reply_id.clone());
        }
        for reply in &node.replies {
            if matches!(reply.author, Author::Person)
                && reply.status == Some(Status::Pending)
                && !reply.message_id.is_empty()
            {
                out.push(reply.message_id.clone());
            }
            for sub in &reply.sub_threads {
                walk(sub, out);
            }
        }
        for sub in &node.sub_threads {
            walk(sub, out);
        }
    }
    let mut out = Vec::new();
    for node in &view.threads {
        walk(node, &mut out);
    }
    // Send ids sort by time (`snd_` and a time-ordered suffix).
    out.sort();
    out.dedup();
    out
}

/// The agent's replies under `node`, at any depth.
pub(crate) fn agent_replies(node: &ThreadNode, out: &mut Vec<String>) {
    for reply in &node.replies {
        if matches!(reply.author, Author::Agent { .. }) {
            out.push(reply.id.clone());
        }
        for sub in &reply.sub_threads {
            agent_replies(sub, out);
        }
    }
    for sub in &node.sub_threads {
        agent_replies(sub, out);
    }
}

/// Who wrote a reply, as people read it.
pub(crate) fn author(reply: &ReplyNode) -> String {
    match &reply.author {
        Author::Person => "You".to_owned(),
        Author::Agent { host, name, .. } => reply
            .author_name
            .clone()
            .or_else(|| name.clone())
            .or_else(|| host.clone())
            .unwrap_or_else(|| "agent".into()),
    }
}
