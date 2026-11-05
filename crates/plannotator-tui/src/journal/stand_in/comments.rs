//! The stand-in's comments, and the view of them in the Inbox's shape: each comment with
//! its replies, and the sub-threads on the words of each, folded once resolved with nothing
//! open under it.

use plannotator_tui_journal::{
    Anchor as On, Author, Parent, Place, ReplyNode, State as ThreadState, Status, ThreadNode,
};

/// A comment as the stand-in keeps it: a comment, a reply under one (`re`), or a sub-thread
/// on a message's words (`parent`).
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools, reason = "independent facts of one comment, as the Inbox keeps them")]
pub(crate) struct Comment {
    pub(crate) id: String,
    pub(crate) on: On,
    pub(crate) quote: String,
    pub(crate) text: String,
    pub(crate) draft: bool,
    pub(crate) status: Option<Status>,
    pub(crate) live: bool,
    pub(crate) resolved: bool,
    pub(crate) re: Option<String>,
    pub(crate) parent: Option<String>,
    /// An agent's reply.
    pub(crate) agent: bool,
}

impl Comment {
    pub(crate) fn draft(id: &str, on: On, quote: &str, text: &str) -> Self {
        Self {
            id: id.into(),
            on,
            quote: quote.into(),
            text: text.into(),
            draft: true,
            status: None,
            live: false,
            resolved: false,
            re: None,
            parent: None,
            agent: false,
        }
    }

    /// A comment the window made on message `message_id`'s `quote`, sent unless `draft`.
    pub(crate) fn on_message(id: &str, message_id: &str, quote: &str, text: &str, draft: bool) -> Self {
        let mut comment = Self::draft(id, On::Message { message_id: message_id.into() }, quote, text);
        comment.draft = draft;
        comment.status = (!draft).then_some(Status::Delivered);
        comment
    }

    /// The agent's reply `id` under comment `under`.
    pub(crate) fn agent_reply(id: &str, under: &Comment, text: &str) -> Self {
        let mut reply = Self::draft(id, under.on.clone(), "", text);
        (reply.draft, reply.agent, reply.re) = (false, true, Some(under.id.clone()));
        reply
    }
}

/// The comments as the Inbox's view lists them.
pub(super) fn view(comments: &[Comment]) -> Vec<ThreadNode> {
    comments.iter().filter(|c| c.re.is_none() && c.parent.is_none()).map(|c| node(c, comments)).collect()
}

fn subs(id: &str, comments: &[Comment]) -> Vec<ThreadNode> {
    let on_it = comments.iter().filter(|c| c.re.is_none() && c.parent.as_deref() == Some(id));
    on_it.map(|c| node(c, comments)).collect()
}

fn node(comment: &Comment, comments: &[Comment]) -> ThreadNode {
    let replies: Vec<ReplyNode> = comments
        .iter()
        .filter(|c| c.re.as_deref() == Some(&comment.id))
        .map(|c| reply(c, comments))
        .collect();
    let sub_threads = subs(&comment.id, comments);
    let under = sub_threads.iter().chain(replies.iter().flat_map(|reply| &reply.sub_threads));
    let folded = comment.resolved && under.clone().all(|sub| sub.folded);
    let parent = comment.parent.clone().map(|id| Parent { id, quote_offset: 0 });
    ThreadNode {
        id: comment.id.clone(),
        reply_id: if comment.draft { String::new() } else { "snd_1".into() },
        round: 1,
        state: if comment.resolved { ThreadState::Resolved } else { ThreadState::Open },
        place: Place { on: comment.on.clone(), quote: comment.quote.clone(), unanchored: false, parent },
        text: comment.text.clone(),
        draft: comment.draft,
        folded,
        status: comment.status,
        live: comment.live,
        replies,
        sub_threads,
        other: serde_json::Map::new(),
    }
}

fn reply(comment: &Comment, comments: &[Comment]) -> ReplyNode {
    let author = if comment.agent {
        Author::Agent {
            host: Some("claude-code".into()),
            session: Some("ses_a".into()),
            name: Some("Claude Code".into()),
        }
    } else {
        Author::Person
    };
    ReplyNode {
        id: comment.id.clone(),
        author,
        round: 1,
        message_id: String::new(),
        text: comment.text.clone(),
        at: "2026-11-05T10:30:00.000Z".into(),
        author_name: comment.agent.then(|| "Claude Code".into()),
        draft: comment.draft,
        status: comment.status,
        sub_threads: subs(&comment.id, comments),
        replies: None,
        other: serde_json::Map::new(),
    }
}
