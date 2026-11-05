//! The person's API of the Inbox, behind one trait: the application's single seam to the
//! Inbox, which its tests replace with a stand-in.
//!
//! Every route is a device-door route (contract E7, analysis § 5.2), served by the window's
//! route of the same name. Routes marked E7 below do not exist on the door yet.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::discovery::discover;
use crate::error::Error;
use crate::http::{CALL_TIMEOUT, Door, agent, segment};
use crate::pairing::{Paired, Token, pair_local, token_path};
use crate::records::{
    AttachmentText, CommentRecord, CommentSend, Deposit, Health, ProjectSessions, Thread, ThreadRow,
};
use crate::requests::{
    AttachmentVersion, IdempotencyKey, NewComment, NewReply, NewSubThread, ReviewTarget, SendMode,
};
use crate::view::ThreadView;
use crate::watch::Watch;

/// The contract major this client reads: a `v` above it is refused, never guessed at.
pub const CONTRACT: u64 = 1;

/// What a terminal does with the Inbox.
pub trait Inbox {
    fn health(&self) -> Result<Health, Error>;
    /// Every live agent session, by project.
    fn sessions(&self) -> Result<Vec<ProjectSessions>, Error>;
    /// The thread list, every section's rows in the list's order.
    fn threads(&self) -> Result<Vec<ThreadRow>, Error>;
    /// One thread with its messages' text.
    fn thread(&self, thread_id: &str) -> Result<Thread, Error>;
    /// The computed view of a thread's comment threads (E7: `threads/:id/comments?v=1`).
    fn thread_view(&self, thread_id: &str) -> Result<ThreadView, Error>;
    /// An attached text file's words.
    fn attachment_text(
        &self,
        attachment_id: &str,
        version: AttachmentVersion,
    ) -> Result<AttachmentText, Error>;
    /// Deposit a review target for a session, in its review thread (E7 with P1: `review-targets`).
    fn deposit(&self, target: &ReviewTarget) -> Result<Deposit, Error>;
    /// A comment on an agent's message's words, new or an edit of a draft.
    fn comment_on_message(&self, message_id: &str, comment: &NewComment) -> Result<CommentRecord, Error>;
    /// A comment on an attached file's words; `version` is what `attachment_text` answered.
    fn comment_on_attachment(
        &self,
        attachment_id: &str,
        version: &str,
        comment: &NewComment,
    ) -> Result<CommentRecord, Error>;
    /// The person's reply under a sent comment (E7: on the door).
    fn reply(&self, comment_id: &str, reply: &NewReply) -> Result<CommentRecord, Error>;
    /// The person's sub-thread on a passage, placed by the server (E7: on the door, by passage).
    fn sub_thread(&self, sub: &NewSubThread) -> Result<CommentRecord, Error>;
    /// Resolve a sent comment, or open it again (E7: on the door).
    fn resolve(&self, comment_id: &str, resolved: bool) -> Result<CommentRecord, Error>;
    /// Remove a draft.
    fn remove(&self, comment_id: &str) -> Result<CommentRecord, Error>;
    /// Send drafts: `now` the ones named, or `round` (Finish) every draft left
    /// (E2 and E7: `threads/:id/comment-sends`). `key` is the press's: the same press tried
    /// again after an error goes under the same key, so it never sends twice.
    fn send(
        &self,
        thread_id: &str,
        mode: SendMode,
        comment_ids: &[String],
        key: &IdempotencyKey,
    ) -> Result<Vec<CommentSend>, Error>;
    /// Follow the event stream from `from` (a seq; `None`: from now), keeping one thread's
    /// records when `thread_id` is given.
    fn watch(&self, from: Option<u64>, thread_id: Option<&str>) -> Result<Watch, Error>;
}

/// The Inbox under one data directory, as one paired terminal.
#[derive(Debug, Clone)]
pub struct Client {
    door: Door,
}

impl Client {
    /// A client of the Inbox under `data_dir`; the registry is read again for every call.
    pub fn new(data_dir: impl Into<PathBuf>, token: Token) -> Self {
        Self { door: Door { agent: agent(CALL_TIMEOUT), data_dir: data_dir.into(), token } }
    }
}

/// Find the Inbox under `data_dir`, then pair this terminal as `name` unless a kept token
/// still works (one revoked or unknown is forgotten and replaced).
pub fn connect(data_dir: &Path, name: &str) -> Result<Client, Error> {
    let found = discover(data_dir)?;
    let path = token_path(data_dir);
    // A file this client cannot read is no pairing: pair again in its place.
    let kept = match Paired::load(&path) {
        Err(Error::Io { .. }) => Paired::forget(&path).map(|()| None)?,
        other => other?,
    };
    if let Some(kept) = kept {
        let client = Client::new(data_dir, kept.token);
        match client.health() {
            Err(Error::Unpaired { .. }) => Paired::forget(&path)?,
            Ok(_) => return Ok(client),
            Err(other) => return Err(other),
        }
    }
    let paired = pair_local(&found, name)?;
    paired.save(&path)?;
    Ok(Client::new(data_dir, paired.token))
}

/// The answer of every comment route: the record written.
#[derive(Deserialize)]
struct Written {
    annotation: CommentRecord,
}

#[derive(Deserialize)]
struct Sections {
    sections: Vec<Section>,
}

#[derive(Deserialize)]
struct Section {
    threads: Vec<ThreadRow>,
}

#[derive(Deserialize)]
struct Projects {
    projects: Vec<ProjectSessions>,
}

#[derive(Deserialize)]
struct ThreadAnswer {
    thread: Thread,
}

#[derive(Deserialize)]
struct Sends {
    sends: Vec<CommentSend>,
}

fn fields(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

impl Inbox for Client {
    fn health(&self) -> Result<Health, Error> {
        self.door.get("health")
    }

    fn sessions(&self) -> Result<Vec<ProjectSessions>, Error> {
        Ok(self.door.get::<Projects>("sessions")?.projects)
    }

    fn threads(&self) -> Result<Vec<ThreadRow>, Error> {
        let list: Sections = self.door.get("threads")?;
        Ok(list.sections.into_iter().flat_map(|section| section.threads).collect())
    }

    fn thread(&self, thread_id: &str) -> Result<Thread, Error> {
        Ok(self.door.get::<ThreadAnswer>(&format!("threads/{}", segment(thread_id)?))?.thread)
    }

    fn thread_view(&self, thread_id: &str) -> Result<ThreadView, Error> {
        let route = format!("threads/{}/comments?v={CONTRACT}", segment(thread_id)?);
        let value = self.door.get_value(&route)?;
        match value.get("v").and_then(Value::as_u64) {
            Some(v) if v > CONTRACT => Err(Error::Newer { what: route, v }),
            Some(_) => serde_json::from_value(value).map_err(|e| Error::unreadable(&route, e)),
            None => Err(Error::unreadable(&route, "no `v`")),
        }
    }

    fn attachment_text(
        &self,
        attachment_id: &str,
        version: AttachmentVersion,
    ) -> Result<AttachmentText, Error> {
        self.door.get(&format!("attachments/{}/view?version={}", segment(attachment_id)?, version.query()))
    }

    fn deposit(&self, target: &ReviewTarget) -> Result<Deposit, Error> {
        let body = serde_json::to_value(target).map_err(|e| Error::unreadable("review-targets", e))?;
        self.door.post("review-targets", fields(body), &IdempotencyKey::new())
    }

    fn comment_on_message(&self, message_id: &str, comment: &NewComment) -> Result<CommentRecord, Error> {
        let route = format!("messages/{}/annotations", segment(message_id)?);
        segment(&comment.id)?;
        let body = json!({ "annotation": comment.annotation() });
        Ok(self.door.post::<Written>(&route, fields(body), &IdempotencyKey::new())?.annotation)
    }

    fn comment_on_attachment(
        &self,
        attachment_id: &str,
        version: &str,
        comment: &NewComment,
    ) -> Result<CommentRecord, Error> {
        segment(&comment.id)?;
        let body = json!({
            "attachment_id": segment(attachment_id)?,
            "version": version,
            "annotation": comment.annotation(),
        });
        Ok(self.door.post::<Written>("annotations", fields(body), &IdempotencyKey::new())?.annotation)
    }

    fn reply(&self, comment_id: &str, reply: &NewReply) -> Result<CommentRecord, Error> {
        let route = format!("annotations/{}/replies", segment(comment_id)?);
        let body = json!({ "annotation": { "id": segment(&reply.id)?, "text": reply.text } });
        Ok(self.door.post::<Written>(&route, fields(body), &IdempotencyKey::new())?.annotation)
    }

    fn sub_thread(&self, sub: &NewSubThread) -> Result<CommentRecord, Error> {
        let route = format!("annotations/{}/subthreads", segment(&sub.on.parent)?);
        let body = json!({ "annotation": {
            "id": segment(&sub.id)?,
            "text": sub.text,
            "quote": sub.on.quote,
            "occurrence": sub.on.occurrence,
        } });
        Ok(self.door.post::<Written>(&route, fields(body), &IdempotencyKey::new())?.annotation)
    }

    fn resolve(&self, comment_id: &str, resolved: bool) -> Result<CommentRecord, Error> {
        let route = format!("annotations/{}/resolve", segment(comment_id)?);
        Ok(self
            .door
            .post::<Written>(&route, fields(json!({ "resolved": resolved })), &IdempotencyKey::new())?
            .annotation)
    }

    fn remove(&self, comment_id: &str) -> Result<CommentRecord, Error> {
        let route = format!("annotations/{}/remove", segment(comment_id)?);
        Ok(self.door.post::<Written>(&route, Map::new(), &IdempotencyKey::new())?.annotation)
    }

    fn send(
        &self,
        thread_id: &str,
        mode: SendMode,
        comment_ids: &[String],
        key: &IdempotencyKey,
    ) -> Result<Vec<CommentSend>, Error> {
        let route = format!("threads/{}/comment-sends", segment(thread_id)?);
        if mode == SendMode::Now && comment_ids.is_empty() {
            return Err(Error::Invalid("a `now` send names the drafts it sends".to_owned()));
        }
        let mut body = fields(json!({ "mode": mode }));
        if !comment_ids.is_empty() {
            let ids: Result<Vec<&str>, Error> = comment_ids.iter().map(|id| segment(id)).collect();
            body.insert("comment_ids".to_owned(), json!(ids?));
        }
        Ok(self.door.post::<Sends>(&route, body, key)?.sends)
    }

    fn watch(&self, from: Option<u64>, thread_id: Option<&str>) -> Result<Watch, Error> {
        Watch::start(self.door.clone(), from, thread_id.map(str::to_owned))
    }
}
