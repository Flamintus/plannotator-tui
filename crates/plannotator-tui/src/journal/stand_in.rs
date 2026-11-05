//! A stand-in for the Inbox, for the app's tests: threads, attached files and comments held
//! in memory, the comments with their replies and sub-threads
//! listed by a view of the Inbox's shape (`comments.rs`). A route it does not hold, or one a test lists in
//! `missing`, answers as an Inbox without that route would.

#![allow(clippy::expect_used, reason = "test fixtures assert by panicking")]

mod comments;

use std::cell::RefCell;

use plannotator_tui_journal::{
    Anchor as On, AttachmentText, AttachmentVersion, CommentRecord, CommentSend, Deposit, Error, Event,
    FileVersion, Health, IdempotencyKey, Inbox, NewComment, NewReply, NewSubThread, Passage, ProjectSessions,
    ReviewTarget, SendMode, SendTarget, Status, Thread, ThreadRow, ThreadView, Watch,
};

pub(crate) use comments::Comment;
use comments::view;

#[derive(Debug, Default)]
pub(crate) struct StandIn {
    pub(crate) state: RefCell<State>,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) threads: Vec<Thread>,
    /// Attached files' words by attachment id.
    pub(crate) files: Vec<(String, AttachmentText)>,
    /// Routes answered as missing.
    pub(crate) missing: Vec<&'static str>,
    pub(crate) comments: Vec<Comment>,
    /// Every write, in order: `comment <id>`, `remove <id>`.
    pub(crate) writes: Vec<String>,
    /// Rounds finished: the round under way is the next one.
    pub(crate) finished: u64,
    /// Every send asked for: its mode, the comments it names and its key.
    pub(crate) sends: Vec<(SendMode, Vec<String>, String)>,
    /// How many sends to come lose their connection.
    pub(crate) failing_sends: usize,
    /// A view to answer as it is, in place of the one the comments make: one computed by
    /// Plannotator's own code (the client crate's contract fixture).
    pub(crate) view: Option<ThreadView>,
    /// Every sub-thread's passage, as asked for.
    pub(crate) passages: Vec<Passage>,
    /// The threads each watch was asked to follow, and the stream of the last one.
    pub(crate) watched: Vec<Option<String>>,
    pub(crate) stream: Option<std::sync::mpsc::Sender<Event>>,
}

/// The record a comment route answers.
fn record(id: &str) -> CommentRecord {
    serde_json::from_value(serde_json::json!({
        "id": id, "thread_id": "thr", "message_id": "", "attachment_id": "", "path": "", "version": "current",
        "annotation": {}, "created_at": "", "updated_at": "", "removed_at": null, "sent_reply_id": null,
    }))
    .expect("a record")
}

fn refused(route: &str, code: &str) -> Error {
    Error::Refused {
        route: route.to_owned(),
        status: 409,
        code: code.to_owned(),
        message: code.replace('_', " "),
        field: None,
    }
}

impl StandIn {
    /// A new comment on `on`, or an edit of the draft of the same id.
    fn write(&self, on: On, comment: &NewComment) -> Result<CommentRecord, Error> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("comment {}", comment.id));
        match state.comments.iter_mut().find(|kept| kept.id == comment.id) {
            Some(kept) if !kept.draft => return Err(refused("annotations", "annotation_closed")),
            Some(kept) => kept.text.clone_from(&comment.text),
            None => state.comments.push(Comment::draft(&comment.id, on, &comment.quote, &comment.text)),
        }
        Ok(record(&comment.id))
    }

    /// The path of attached file `attachment_id`, as its message lists it.
    fn path_of(&self, attachment_id: &str) -> String {
        let state = self.state.borrow();
        let files = state
            .threads
            .iter()
            .flat_map(|thread| &thread.messages)
            .filter_map(|m| m.other.get("attachments"));
        let found =
            files.filter_map(serde_json::Value::as_array).flatten().find(|file| file["id"] == attachment_id);
        found.and_then(|file| file["path"].as_str()).unwrap_or_default().to_owned()
    }

    fn missing(&self, route: &'static str) -> Result<(), Error> {
        if self.state.borrow().missing.contains(&route) { missing(route) } else { Ok(()) }
    }
}

/// A thread of `messages`: `(id, written by the agent, body, attached files as JSON)`.
pub(crate) fn thread(id: &str, subject: &str, messages: &[(&str, bool, &str, serde_json::Value)]) -> Thread {
    let messages: Vec<serde_json::Value> = messages
        .iter()
        .enumerate()
        .map(|(n, (message, agent, body, files))| {
            let author = if *agent {
                serde_json::json!({ "kind": "agent", "host": "claude-code", "session": "ses_a", "name": null })
            } else {
                serde_json::json!({ "kind": "person" })
            };
            serde_json::json!({
                "id": message, "thread_id": id, "reply_to": null, "author": author, "body": body,
                "created_at": format!("2026-11-05T10:0{n}:00.000Z"), "attachments": files,
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "thread_id": id, "subject": subject, "thread_name": null, "resolved_at": null, "messages": messages,
    }))
    .expect("a thread")
}

/// An attached file as a message lists it.
pub(crate) fn attached(id: &str, name: &str, kind: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "path": format!("/repo/{name}"), "name": name, "kind": kind })
}

fn missing<T>(route: &str) -> Result<T, Error> {
    Err(Error::RouteMissing { route: route.to_owned() })
}

impl Inbox for StandIn {
    fn health(&self) -> Result<Health, Error> {
        missing("health")
    }

    fn sessions(&self) -> Result<Vec<ProjectSessions>, Error> {
        missing("sessions")
    }

    fn threads(&self) -> Result<Vec<ThreadRow>, Error> {
        missing("threads")
    }

    fn thread(&self, thread_id: &str) -> Result<Thread, Error> {
        let state = self.state.borrow();
        let found = state.threads.iter().find(|thread| thread.thread_id == thread_id).cloned();
        found.ok_or_else(|| Error::Refused {
            route: format!("threads/{thread_id}"),
            status: 404,
            code: "thread_not_found".into(),
            message: "no such thread".into(),
            field: None,
        })
    }

    fn thread_view(&self, _thread_id: &str) -> Result<ThreadView, Error> {
        self.missing("threads/:id/comments")?;
        let state = self.state.borrow();
        if let Some(view) = &state.view {
            return Ok(view.clone());
        }
        Ok(ThreadView {
            v: 1,
            round: state.finished + 1,
            threads: view(&state.comments),
            other: serde_json::Map::new(),
        })
    }

    fn attachment_text(
        &self,
        attachment_id: &str,
        _version: AttachmentVersion,
    ) -> Result<AttachmentText, Error> {
        let state = self.state.borrow();
        let found = state.files.iter().find(|(id, _)| id == attachment_id).map(|(_, text)| text.clone());
        found.ok_or_else(|| Error::Refused {
            route: format!("attachments/{attachment_id}/view"),
            status: 404,
            code: "attachment_not_found".into(),
            message: "no such file".into(),
            field: None,
        })
    }

    fn deposit(&self, _target: &ReviewTarget) -> Result<Deposit, Error> {
        missing("review-targets")
    }

    fn comment_on_message(&self, message_id: &str, comment: &NewComment) -> Result<CommentRecord, Error> {
        self.missing("messages/:id/annotations")?;
        self.write(On::Message { message_id: message_id.into() }, comment)
    }

    fn comment_on_attachment(
        &self,
        attachment_id: &str,
        version: &str,
        comment: &NewComment,
    ) -> Result<CommentRecord, Error> {
        let path = self.path_of(attachment_id);
        let version = if version == "current" { FileVersion::Current } else { FileVersion::Sent };
        self.write(On::File { path, version, lines: None, at: None }, comment)
    }

    /// A reply under a sent comment: a draft, or sent at once when the thread is live.
    fn reply(&self, comment_id: &str, reply: &NewReply) -> Result<CommentRecord, Error> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("reply {} under {comment_id}", reply.id));
        let Some(under) = state.comments.iter().find(|comment| comment.id == comment_id && !comment.draft)
        else {
            return Err(refused("annotations/:id/replies", "annotation_not_allowed"));
        };
        let mut kept = Comment::draft(&reply.id, under.on.clone(), "", &reply.text);
        kept.re = Some(comment_id.to_owned());
        if under.live {
            kept.draft = false;
            kept.status = Some(Status::Pending);
        }
        state.comments.push(kept);
        Ok(record(&reply.id))
    }

    /// A sub-thread on a passage of a message of a comment's thread: a draft.
    fn sub_thread(&self, sub: &NewSubThread) -> Result<CommentRecord, Error> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("sub-thread {} on {}", sub.id, sub.on.parent));
        state.passages.push(sub.on.clone());
        let Some(on) =
            state.comments.iter().find(|comment| comment.id == sub.on.parent).map(|c| c.on.clone())
        else {
            return Err(refused("annotations/:id/subthreads", "message_not_found"));
        };
        let mut kept = Comment::draft(&sub.id, on, &sub.on.quote, &sub.text);
        kept.parent = Some(sub.on.parent.clone());
        state.comments.push(kept);
        Ok(record(&sub.id))
    }

    fn resolve(&self, comment_id: &str, resolved: bool) -> Result<CommentRecord, Error> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("resolve {comment_id} {resolved}"));
        let Some(kept) = state.comments.iter_mut().find(|comment| comment.id == comment_id) else {
            return Err(refused("annotations/:id/resolve", "annotation_not_found"));
        };
        kept.resolved = resolved;
        Ok(record(comment_id))
    }

    fn remove(&self, comment_id: &str) -> Result<CommentRecord, Error> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("remove {comment_id}"));
        let Some(index) = state.comments.iter().position(|comment| comment.id == comment_id) else {
            return Err(refused("annotations/:id/remove", "annotation_not_found"));
        };
        if state.comments.get(index).is_some_and(|comment| !comment.draft) {
            return Err(refused("annotations/:id/remove", "annotation_closed"));
        }
        state.comments.remove(index);
        Ok(record(comment_id))
    }

    /// `now` sends the drafts named and makes their threads live; `round` sends every draft
    /// left and finishes the round.
    fn send(
        &self,
        thread_id: &str,
        mode: SendMode,
        ids: &[String],
        key: &IdempotencyKey,
    ) -> Result<Vec<CommentSend>, Error> {
        self.missing("threads/:id/comment-sends")?;
        let mut state = self.state.borrow_mut();
        state.sends.push((mode, ids.to_vec(), key.as_str().to_owned()));
        if state.failing_sends > 0 {
            state.failing_sends -= 1;
            return Err(Error::Transport {
                route: "threads/:id/comment-sends".into(),
                detail: "connection reset".into(),
            });
        }
        let round = state.finished + 1;
        let mut carried = Vec::new();
        for comment in
            state.comments.iter_mut().filter(|c| c.draft && (mode == SendMode::Round || ids.contains(&c.id)))
        {
            comment.draft = false;
            comment.status = Some(Status::Pending);
            comment.live = mode == SendMode::Now;
            carried.push(comment.id.clone());
        }
        if mode == SendMode::Round {
            state.finished += 1;
        }
        let to = SendTarget { host: Some("claude-code".into()), session: "ses_a".into() };
        Ok(vec![CommentSend {
            id: "snd_1".into(),
            thread_id: thread_id.into(),
            batch: "b_1".into(),
            to,
            round,
            mode,
            comments: carried,
            created_at: String::new(),
            other: serde_json::Map::new(),
        }])
    }

    fn watch(&self, _from: Option<u64>, thread_id: Option<&str>) -> Result<Watch, Error> {
        self.missing("events")?;
        let (events, stream) = std::sync::mpsc::channel();
        let mut state = self.state.borrow_mut();
        state.watched.push(thread_id.map(str::to_owned));
        state.stream = Some(events);
        Ok(Watch::from_events(stream))
    }
}
