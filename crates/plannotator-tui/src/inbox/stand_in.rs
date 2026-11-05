//! A stand-in for the Inbox, for the app's tests: threads, attached files, live sessions
//! and deposits held in memory. A route it does not hold, or one a test lists in
//! `missing`, answers as an Inbox without that route would.

#![allow(clippy::expect_used, reason = "test fixtures assert by panicking")]

use std::cell::RefCell;

use plannotator_tui_inbox::{
    AttachmentText, AttachmentVersion, CommentRecord, CommentSend, Deposit, Error, Health, IdempotencyKey,
    Inbox, NewComment, NewReply, NewSubThread, ProjectSessions, ReviewTarget, SendMode, Target, Thread,
    ThreadRow, ThreadView, Watch,
};

#[derive(Debug, Default)]
pub(crate) struct StandIn {
    pub(crate) state: RefCell<State>,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) threads: Vec<Thread>,
    /// Attached files' words by attachment id.
    pub(crate) files: Vec<(String, AttachmentText)>,
    pub(crate) sessions: Vec<ProjectSessions>,
    /// Every deposit asked for, and where it landed.
    pub(crate) deposits: Vec<(ReviewTarget, Deposit)>,
    /// Routes answered as missing.
    pub(crate) missing: Vec<&'static str>,
}

impl StandIn {
    /// An Inbox with one live session, `ses_a` of `claude-code`, in `/repo`.
    pub(crate) fn with_session() -> Self {
        let sessions = serde_json::from_value(serde_json::json!([{
            "project": { "id": "prj_1", "name": "repo", "root": "/repo" },
            "sessions": [{ "session": "ses_a", "host": "claude-code", "busy": false, "last_thread": null }],
        }]));
        let inbox = Self::default();
        inbox.state.borrow_mut().sessions = sessions.expect("sessions");
        inbox
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
        self.missing("sessions")?;
        Ok(self.state.borrow().sessions.clone())
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
        missing("threads/:id/comments")
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

    /// One review thread for all deposits; the same words again land on the same message.
    fn deposit(&self, target: &ReviewTarget) -> Result<Deposit, Error> {
        self.missing("review-targets")?;
        let mut state = self.state.borrow_mut();
        if let Some((_, landed)) = state.deposits.iter().find(|(asked, _)| asked == target) {
            let landed = Deposit { created: false, ..landed.clone() };
            state.deposits.push((target.clone(), landed.clone()));
            return Ok(landed);
        }
        let Target::ChatMessage { text, .. } = &target.target else { return missing("review-targets") };
        let message_id = format!("msg_r{}", state.deposits.len() + 1);
        if !state.threads.iter().any(|thread| thread.thread_id == "thr_review") {
            state.threads.push(thread("thr_review", "Review", &[]));
        }
        let message = thread("thr_review", "Review", &[(&message_id, true, text, serde_json::json!([]))]);
        if let Some(review) = state.threads.iter_mut().find(|thread| thread.thread_id == "thr_review") {
            review.messages.extend(message.messages);
        }
        let landed = Deposit {
            thread_id: "thr_review".into(),
            message_id,
            attachment_id: None,
            created: true,
            other: serde_json::Map::new(),
        };
        state.deposits.push((target.clone(), landed.clone()));
        Ok(landed)
    }

    fn comment_on_message(&self, _message_id: &str, _comment: &NewComment) -> Result<CommentRecord, Error> {
        missing("messages/:id/annotations")
    }

    fn comment_on_attachment(&self, _: &str, _: &str, _: &NewComment) -> Result<CommentRecord, Error> {
        missing("annotations")
    }

    fn reply(&self, _comment_id: &str, _reply: &NewReply) -> Result<CommentRecord, Error> {
        missing("annotations/:id/replies")
    }

    fn sub_thread(&self, _sub: &NewSubThread) -> Result<CommentRecord, Error> {
        missing("annotations/:id/subthreads")
    }

    fn resolve(&self, _comment_id: &str, _resolved: bool) -> Result<CommentRecord, Error> {
        missing("annotations/:id/resolve")
    }

    fn remove(&self, _comment_id: &str) -> Result<CommentRecord, Error> {
        missing("annotations/:id/remove")
    }

    fn send(
        &self,
        _: &str,
        _: SendMode,
        _: &[String],
        _: &IdempotencyKey,
    ) -> Result<Vec<CommentSend>, Error> {
        missing("threads/:id/comment-sends")
    }

    fn watch(&self, _from: Option<u64>, _thread_id: Option<&str>) -> Result<Watch, Error> {
        missing("events")
    }
}
