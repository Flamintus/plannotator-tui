//! One reader of the event stream: connects, follows the frames, reconnects after a cut.

use std::io::{BufRead as _, BufReader, Read as _};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::{Event, Kind, Record};
use crate::client::CONTRACT;
use crate::error::Error;
use crate::http::{self, Door};
use crate::sse::{Frame, Parser};

/// No byte for two ping periods: the connection is taken as cut.
const IDLE: Duration = Duration::from_secs(30);
/// The reconnection delay until the stream names one (the Inbox's `retry: 2000`).
const RETRY: Duration = Duration::from_secs(2);
const CONNECT: Duration = Duration::from_secs(2);
/// The longest line read: an Inbox line carries one record, far below this.
const LINE_MAX: u64 = 64 * 1024 * 1024;

/// How one connection ended.
enum End {
    /// The watch was dropped.
    Gone,
    Fatal(Error),
    Broken(Error),
}

pub(super) struct Reader {
    door: Door,
    agent: ureq::Agent,
    filter: Option<String>,
    /// The last seq seen: what `Last-Event-ID` resumes from.
    position: Option<u64>,
    /// The `serverSession` of the last hello.
    session: Option<String>,
    retry: Duration,
    raw: Sender<Event>,
    stop: Receiver<()>,
}

#[derive(Deserialize)]
struct Hello {
    #[serde(rename = "serverSession")]
    server_session: String,
    cursor: u64,
    v: Option<u64>,
}

#[derive(Deserialize)]
struct Line {
    seq: u64,
    kind: String,
    id: String,
    v: Option<u64>,
    thread_id: Option<String>,
    message: Option<Carried>,
    annotation: Option<Carried>,
}

/// The record a `message` or `annotation` line carries, for its thread.
#[derive(Deserialize)]
struct Carried {
    thread_id: Option<String>,
}

impl Reader {
    pub(super) fn new(
        door: Door,
        from: Option<u64>,
        filter: Option<String>,
        raw: Sender<Event>,
        stop: Receiver<()>,
    ) -> Self {
        // No deadline on the whole call, which never ends; a read that waits too long is a cut.
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .timeout_connect(Some(CONNECT))
            .timeout_recv_response(Some(http::CALL_TIMEOUT))
            .timeout_per_read(Some(IDLE))
            .build()
            .into();
        Self { door, agent, filter, position: from, session: None, retry: RETRY, raw, stop }
    }

    pub(super) fn run(mut self) {
        loop {
            match self.connect() {
                End::Gone => return,
                End::Fatal(error) => {
                    let _ = self.raw.send(Event::Stopped(error));
                    return;
                }
                End::Broken(error) => {
                    if self.raw.send(Event::Disconnected(error)).is_err() {
                        return;
                    }
                }
            }
            if !matches!(self.stop.recv_timeout(self.retry), Err(RecvTimeoutError::Timeout)) {
                return;
            }
        }
    }

    fn connect(&mut self) -> End {
        let route = "events";
        let url = match self.door.url(route) {
            Ok(url) => url,
            Err(error) => return End::Broken(error),
        };
        let mut request = self
            .agent
            .get(url)
            .header("Authorization", self.door.bearer())
            .header("Accept", "text/event-stream");
        if let Some(seq) = self.position {
            request = request.header("Last-Event-ID", seq.to_string());
        }
        let mut response = match request.call() {
            Ok(response) => response,
            Err(error) => return End::Broken(Error::transport(route, error)),
        };
        let status = response.status().as_u16();
        if status != 200 {
            let text = http::body_text(&mut response, route).unwrap_or_default();
            return match Error::refusal(route, status, &text) {
                error @ (Error::Unpaired { .. } | Error::RouteMissing { .. }) => End::Fatal(error),
                error => End::Broken(error),
            };
        }
        let mut body = BufReader::new(response.into_body().into_reader());
        let mut parser = Parser::default();
        let mut line = Vec::new();
        loop {
            if matches!(self.stop.try_recv(), Err(TryRecvError::Disconnected)) {
                return End::Gone;
            }
            line.clear();
            match body.by_ref().take(LINE_MAX).read_until(b'\n', &mut line) {
                Ok(0) => return End::Broken(Error::transport(route, "the Inbox closed the event stream")),
                Ok(read) if read as u64 == LINE_MAX && !line.ends_with(b"\n") => {
                    return End::Fatal(Error::unreadable(route, "a line longer than this client reads"));
                }
                Ok(_) => {}
                Err(error) => return End::Broken(Error::transport(route, error)),
            }
            if let Some(frame) = parser.line(&String::from_utf8_lossy(&line)) {
                if let Some(retry) = frame.retry {
                    self.retry = Duration::from_millis(retry);
                }
                if let Err(end) = self.frame(&frame) {
                    return end;
                }
            }
        }
    }

    fn frame(&mut self, frame: &Frame) -> Result<(), End> {
        match frame.event.as_str() {
            "hello" => {
                let hello: Hello = read(frame)?;
                newer(hello.v, "the event stream's hello")?;
                let cursor = hello.cursor;
                self.position.get_or_insert(cursor);
                let event = match self.session.replace(hello.server_session.clone()) {
                    Some(known) if known != hello.server_session => {
                        Event::Restarted { server_session: hello.server_session, cursor }
                    }
                    _ => Event::Open { cursor },
                };
                self.raw.send(event).map_err(|_| End::Gone)
            }
            "record" => {
                let line: Line = read(frame)?;
                newer(line.v, &format!("event record {}", line.seq))?;
                // A line already told (a replay that overlaps) is never told twice.
                if self.position.is_some_and(|seen| line.seq <= seen) {
                    return Ok(());
                }
                self.position = Some(line.seq);
                let Some(kind) = Kind::from_wire(&line.kind) else { return Ok(()) };
                let carried = line.message.or(line.annotation).and_then(|c| c.thread_id);
                let thread_id = line.thread_id.or(carried);
                if self.filter.as_ref().is_some_and(|watched| thread_id.as_ref() != Some(watched)) {
                    return Ok(());
                }
                let record = Record { seq: line.seq, kind, id: line.id, thread_id };
                self.raw.send(Event::Records(vec![record])).map_err(|_| End::Gone)
            }
            _ => Ok(()),
        }
    }
}

fn read<T: DeserializeOwned>(frame: &Frame) -> Result<T, End> {
    serde_json::from_str(&frame.data).map_err(|e| End::Fatal(Error::unreadable("events", e)))
}

fn newer(v: Option<u64>, what: &str) -> Result<(), End> {
    match v {
        Some(v) if v > CONTRACT => Err(End::Fatal(Error::Newer { what: what.to_owned(), v })),
        _ => Ok(()),
    }
}
