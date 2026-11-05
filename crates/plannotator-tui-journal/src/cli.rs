//! The review journal behind one trait, the application's single seam to it: [`Cli`] runs
//! Plannotator's CLI, an application's tests put a stand-in.
//!
//! Every call is `plannotator threads <action> -`, the call's JSON object on stdin (a
//! comment's words can be longer than a command line takes), naming the review by its
//! target. stdout carries one JSON value; a refusal is `<code>: <message>` on stderr.

use std::ffi::OsStr;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::error::Error;
use crate::records::{Sent, Viewed};
use crate::requests::{IdempotencyKey, NewComment, ReviewTarget, SendMode};

/// The thread view's contract this client reads: a `v` above it is refused, never guessed at.
pub const CONTRACT: u64 = 1;

/// How long a call may take before it is stopped. A call takes a few hundred milliseconds;
/// one that waits on the journal's lock gives up on its own after eight seconds.
const TIMEOUT: Duration = Duration::from_secs(15);

/// What `threads --help` prints when the CLI has the client's actions.
const CLIENT_USAGE: &str = "plannotator threads <action>";

/// What changes when a review's journal is written: its size and modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    size: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    /// A stamp from a journal's size and time.
    pub fn new(size: u64, modified: Option<SystemTime>) -> Self {
        Self { size, modified }
    }
}

/// What a terminal does with a review's journal. Each review is named by its target; a write
/// to a review with no journal yet makes one.
pub trait Journal {
    /// The review's key and thread view, the person's drafts included: an empty view for a
    /// review with no journal yet.
    fn view(&self, target: &ReviewTarget) -> Result<Viewed, Error>;
    /// The person's comment, reply or sub-thread, as a draft; the same id again edits it.
    fn comment(&self, target: &ReviewTarget, comment: &NewComment) -> Result<(), Error>;
    /// Remove the person's draft, and everything of theirs under it.
    fn remove(&self, target: &ReviewTarget, id: &str) -> Result<(), Error>;
    /// Resolve a sent comment or sub-thread, or open it again.
    fn resolve(&self, target: &ReviewTarget, id: &str, resolved: bool) -> Result<(), Error>;
    /// Send drafts to the agent's session: `now` the ones named, or `round` every draft left,
    /// ending the round. `key` is the press's: the same press tried again goes under the
    /// same key, and the journal gives the first send back, its text included.
    fn send(
        &self,
        target: &ReviewTarget,
        mode: SendMode,
        ids: &[String],
        key: &IdempotencyKey,
    ) -> Result<Sent, Error>;
    /// The client handed these sends to the agent's session itself.
    fn delivered(&self, target: &ReviewTarget, sends: &[String]) -> Result<(), Error>;
    /// When review `review` (its key) was last written, without running anything; `None`
    /// when it has no journal, or this journal cannot tell.
    fn stamp(&self, review: &str) -> Option<Stamp>;
}

/// Plannotator's CLI, writing the journals under one data directory.
#[derive(Debug, Clone)]
pub struct Cli {
    program: PathBuf,
    data_dir: PathBuf,
    timeout: Duration,
}

impl Cli {
    /// `program`, the `plannotator` to run, over the journals under `data_dir`, which the
    /// CLI is given as `PLANNOTATOR_DATA_DIR` so both read the same folder.
    pub fn new(program: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self { program: program.into(), data_dir: data_dir.into(), timeout: TIMEOUT }
    }

    /// The same, a call stopped after `timeout`.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The first `plannotator` on `path`, a `PATH` value.
    pub fn find(path: &OsStr) -> Option<PathBuf> {
        let name = if cfg!(windows) { "plannotator.exe" } else { "plannotator" };
        std::env::split_paths(path).map(|dir| dir.join(name)).find(|candidate| runnable(candidate))
    }

    /// Whether this Plannotator has the review journal's client actions: its
    /// `threads --help` names them. A Plannotator without them never gets a call.
    pub fn probe(&self) -> Result<(), Error> {
        let done = self.run("--help", &["threads", "--help"], &[])?;
        if done.status.success() && String::from_utf8_lossy(&done.stdout).contains(CLIENT_USAGE) {
            return Ok(());
        }
        Err(Error::Usage {
            action: "--help".to_owned(),
            detail: format!("{} has no review journal (`threads` actions)", self.program.display()),
        })
    }

    fn call<T: DeserializeOwned>(
        &self,
        action: &str,
        target: &ReviewTarget,
        mut fields: Map<String, Value>,
    ) -> Result<T, Error> {
        fields.insert("target".to_owned(), json!(target));
        let input = Value::Object(fields).to_string();
        let done = self.run(action, &["threads", action, "-"], input.as_bytes())?;
        let said = || first_line(&done.stderr);
        match done.status.code() {
            Some(0) => serde_json::from_slice(&done.stdout).map_err(|e| Error::unreadable(action, e)),
            Some(1) => Err(Error::refusal(action, &String::from_utf8_lossy(&done.stderr))),
            Some(2) => Err(Error::Usage { action: action.to_owned(), detail: said() }),
            Some(code) => {
                Err(Error::Failed { action: action.to_owned(), detail: format!("exit {code}: {}", said()) })
            }
            None => {
                Err(Error::Failed { action: action.to_owned(), detail: "stopped by a signal".to_owned() })
            }
        }
    }

    /// Run the program with `args`, `input` on its stdin, stopping it after the timeout.
    fn run(&self, action: &str, args: &[&str], input: &[u8]) -> Result<Finished, Error> {
        let mut child = Command::new(&self.program)
            .args(args)
            .env("PLANNOTATOR_DATA_DIR", &self.data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Spawn {
                program: self.program.display().to_string(),
                detail: e.to_string(),
            })?;
        // Each pipe on a thread of its own: a child that writes before it reads, or never
        // reads, cannot hold this one past the timeout.
        let writer = child.stdin.take().map(|mut pipe| {
            let input = input.to_vec();
            std::thread::spawn(move || drop(pipe.write_all(&input)))
        });
        let stdout = child.stdout.take().map(drain);
        let stderr = child.stderr.take().map(drain);
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < self.timeout => std::thread::sleep(Duration::from_millis(5)),
                Ok(None) => {
                    drop(child.kill());
                    drop(child.wait());
                    return Err(Error::Timeout { action: action.to_owned() });
                }
                Err(e) => return Err(Error::Failed { action: action.to_owned(), detail: e.to_string() }),
            }
        };
        if let Some(writer) = writer {
            drop(writer.join());
        }
        Ok(Finished { status, stdout: joined(stdout), stderr: joined(stderr) })
    }
}

/// What a finished run gave.
struct Finished {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn drain(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        drop(pipe.read_to_end(&mut out));
        out
    })
}

fn joined(reader: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader.and_then(|reader| reader.join().ok()).unwrap_or_default()
}

fn first_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().trim().to_owned()
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.is_file()
}

/// Whether `review` reads as a review's key (`last-…`, `file-…`, `plan-…` and 24 hex digits):
/// only such a name becomes a path.
fn is_key(review: &str) -> bool {
    let Some((kind, hash)) = review.split_once('-') else { return false };
    matches!(kind, "last" | "file" | "plan")
        && hash.len() == 24
        && hash.chars().all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
}

/// The journal of review `review` under `data_dir`, as Plannotator lays it out.
fn journal_path(data_dir: &Path, review: &str) -> Option<PathBuf> {
    is_key(review).then(|| data_dir.join("reviews").join(review).join("journal.jsonl"))
}

impl Journal for Cli {
    fn view(&self, target: &ReviewTarget) -> Result<Viewed, Error> {
        let answer: Value = self.call("view", target, Map::new())?;
        match answer.pointer("/view/v").and_then(Value::as_u64) {
            Some(v) if v > CONTRACT => Err(Error::Newer { what: "the thread view".to_owned(), v }),
            Some(_) => serde_json::from_value(answer).map_err(|e| Error::unreadable("view", e)),
            None => Err(Error::unreadable("view", "no `view.v`")),
        }
    }

    fn comment(&self, target: &ReviewTarget, comment: &NewComment) -> Result<(), Error> {
        self.call::<Value>("comment", target, comment.fields()).map(drop)
    }

    fn remove(&self, target: &ReviewTarget, id: &str) -> Result<(), Error> {
        self.call::<Value>("remove", target, fields(json!({ "id": id }))).map(drop)
    }

    fn resolve(&self, target: &ReviewTarget, id: &str, resolved: bool) -> Result<(), Error> {
        let action = if resolved { "resolve" } else { "reopen" };
        self.call::<Value>(action, target, fields(json!({ "id": id }))).map(drop)
    }

    fn send(
        &self,
        target: &ReviewTarget,
        mode: SendMode,
        ids: &[String],
        key: &IdempotencyKey,
    ) -> Result<Sent, Error> {
        if mode == SendMode::Now && ids.is_empty() {
            return Err(Error::Invalid("a `now` send names the drafts it sends".to_owned()));
        }
        let mut body = fields(json!({ "mode": mode, "idempotency_key": key.as_str() }));
        if mode == SendMode::Now {
            body.insert("ids".to_owned(), json!(ids));
        }
        self.call("send", target, body)
    }

    fn delivered(&self, target: &ReviewTarget, sends: &[String]) -> Result<(), Error> {
        self.call::<Value>("delivered", target, fields(json!({ "sends": sends }))).map(drop)
    }

    fn stamp(&self, review: &str) -> Option<Stamp> {
        let meta = std::fs::metadata(journal_path(&self.data_dir, review)?).ok()?;
        Some(Stamp::new(meta.len(), meta.modified().ok()))
    }
}

fn fields(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

#[cfg(test)]
mod tests;
