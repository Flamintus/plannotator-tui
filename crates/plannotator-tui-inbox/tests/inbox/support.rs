//! A stand-in Inbox for the tests: a loopback HTTP server that answers what each test says,
//! records every request, and writes the registry (`inbox/inbox.json`) into a fresh data
//! directory under the temp dir. No test reaches a real Inbox or a real data directory.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use crate::Result;

pub(crate) const SERVER_SESSION: &str = "ss_one";
pub(crate) const CONNECTION_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

pub(crate) enum Reply {
    Json(u16, Value),
    /// An event stream: these bytes, then the connection closes, or (`hold`) stays open
    /// with a ping every 50 ms until the client leaves.
    Stream {
        text: String,
        hold: bool,
    },
    /// Close the connection without an answer.
    Hang,
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

pub(crate) struct FakeInbox {
    pub(crate) data_dir: PathBuf,
    pub(crate) port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
}

/// A fresh, empty directory for one test.
pub(crate) fn scratch(name: &str) -> Result<PathBuf> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("plannotator-tui-inbox-{}-{name}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The registry an Inbox on `port` writes.
pub(crate) fn write_registry(data_dir: &std::path::Path, port: u16, server_session: &str) -> Result {
    let dir = data_dir.join("inbox");
    std::fs::create_dir_all(&dir)?;
    let entry = json!({
        "v": 1, "pid": 4242, "port": port, "url": format!("http://localhost:{port}"), "version": "0.28.49-test",
        "token": CONNECTION_TOKEN, "serverSession": server_session, "startedAt": "2026-11-05T10:00:00.000Z",
    });
    std::fs::write(dir.join("inbox.json"), entry.to_string())?;
    Ok(())
}

/// The window's health answer for `server_session`.
pub(crate) fn health(server_session: &str) -> Reply {
    Reply::Json(
        200,
        json!({ "ok": true, "app": "plannotator-inbox", "version": "0.28.49-test", "serverSession": server_session, "pid": 4242, "update": null }),
    )
}

impl FakeInbox {
    pub(crate) fn start(
        name: &str,
        handler: impl Fn(&Request) -> Reply + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::serve(scratch(name)?, handler)
    }

    /// Serve under an existing data directory (a second Inbox, after a restart).
    pub(crate) fn serve(
        data_dir: PathBuf,
        handler: impl Fn(&Request) -> Reply + Send + Sync + 'static,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        write_registry(&data_dir, port, SERVER_SESSION)?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let handler: Arc<Handler> = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (seen, handler) = (seen.clone(), handler.clone());
                std::thread::spawn(move || answer(stream, &seen, handler.as_ref()));
            }
        });
        Ok(Self { data_dir, port, requests })
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The requests to one path (query included), in arrival order.
    pub(crate) fn requests_to(&self, path: &str) -> Vec<Request> {
        self.requests().into_iter().filter(|r| r.path == path).collect()
    }
}

fn answer(stream: TcpStream, seen: &Mutex<Vec<Request>>, handler: &Handler) {
    let Some(request) = read_request(&stream) else { return };
    seen.lock().unwrap_or_else(PoisonError::into_inner).push(request.clone());
    let mut stream = stream;
    match handler(&request) {
        Reply::Json(status, body) => {
            let body = body.to_string();
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body.as_bytes()));
        }
        Reply::Stream { text, hold } => {
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            if stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(text.as_bytes())).is_err() {
                return;
            }
            while hold && stream.write_all(b": ping\n\n").and_then(|()| stream.flush()).is_ok() {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Reply::Hang => {}
    }
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next()?.to_owned(), parts.next()?.to_owned());
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (key, value) = header.split_once(':')?;
        headers.push((key.trim().to_owned(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request { method, path, headers, body: String::from_utf8_lossy(&body).into_owned() })
}
