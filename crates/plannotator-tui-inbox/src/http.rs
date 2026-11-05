//! HTTP to the Inbox's device door, `/api/inbox/device/<route>`, on the loopback port the
//! registry names, read again for every request.
//!
//! The door's rules (`packages/server/inbox-devices.ts`): no `Origin`, a bearer device
//! token, a route on its allowlist, and an `idempotency_key` on every POST, under which the
//! door keeps the answer per device and replays it (`Idempotent-Replayed: true`). So a
//! write tries again once, with the same key, when the request did not complete.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::discovery::read_registry;
use crate::error::Error;
use crate::pairing::Token;

pub(crate) const DOOR_PREFIX: &str = "/api/inbox/device/";
/// A whole call, body included; the event stream has its own rules (`watch`).
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Bigger than any answer the Inbox gives: a text view is capped at 16 MiB before JSON.
const BODY_LIMIT: u64 = 64 * 1024 * 1024;
/// The pause before a write that did not complete is sent again.
const RETRY_AFTER: Duration = Duration::from_millis(200);

/// An agent for loopback calls: refusals come back as answers so their envelope can be
/// read, and no proxy from the environment ever sees a token.
pub(crate) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .build()
        .into()
}

pub(crate) type Response = ureq::http::Response<ureq::Body>;

pub(crate) fn body_text(response: &mut Response, route: &str) -> Result<String, Error> {
    response
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(|e| Error::transport(route, e))
}

/// A fresh idempotency key: unique per process and call, so unique per device over time.
pub(crate) fn idempotency_key() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("tui_{nanos:x}_{:x}_{:x}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// An id as one path segment, under the Inbox's own rule for ids (`^[A-Za-z0-9_.:-]{1,200}$`,
/// `packages/core/inbox-attachments.ts`), and never a dot segment, which a URL parser would
/// resolve into another route.
pub(crate) fn segment(id: &str) -> Result<&str, Error> {
    let safe = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-');
    let ok = (1..=200).contains(&id.len()) && id.bytes().all(safe) && id.bytes().any(|b| b != b'.');
    if ok { Ok(id) } else { Err(Error::InvalidId(id.to_owned())) }
}

/// The door of the Inbox under one data directory, as one paired device.
#[derive(Debug, Clone)]
pub(crate) struct Door {
    pub(crate) agent: ureq::Agent,
    pub(crate) data_dir: std::path::PathBuf,
    pub(crate) token: Token,
}

impl Door {
    pub(crate) fn url(&self, route: &str) -> Result<String, Error> {
        Ok(format!("{}{DOOR_PREFIX}{route}", read_registry(&self.data_dir)?.base_url()))
    }

    pub(crate) fn bearer(&self) -> String {
        format!("Bearer {}", self.token.secret())
    }

    pub(crate) fn get<T: DeserializeOwned>(&self, route: &str) -> Result<T, Error> {
        let value = self.get_value(route)?;
        serde_json::from_value(value).map_err(|e| Error::unreadable(route, e))
    }

    pub(crate) fn get_value(&self, route: &str) -> Result<Value, Error> {
        let mut response = self
            .agent
            .get(self.url(route)?)
            .header("Authorization", self.bearer())
            .header("Accept", "application/json")
            .call()
            .map_err(|e| Error::transport(route, e))?;
        answer(route, &mut response)
    }

    /// A write: `fields` plus a fresh `idempotency_key`, sent again once under the same
    /// key when the first request did not complete.
    pub(crate) fn post<T: DeserializeOwned>(
        &self,
        route: &str,
        fields: Map<String, Value>,
    ) -> Result<T, Error> {
        let mut fields = fields;
        fields.insert("idempotency_key".to_owned(), Value::String(idempotency_key()));
        let body = Value::Object(fields).to_string();
        let value = match self.post_once(route, &body) {
            Err(Error::Transport { .. }) => {
                std::thread::sleep(RETRY_AFTER);
                self.post_once(route, &body)?
            }
            other => other?,
        };
        serde_json::from_value(value).map_err(|e| Error::unreadable(route, e))
    }

    fn post_once(&self, route: &str, body: &str) -> Result<Value, Error> {
        let mut response = self
            .agent
            .post(self.url(route)?)
            .header("Authorization", self.bearer())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| Error::transport(route, e))?;
        answer(route, &mut response)
    }
}

/// A success's JSON, or the refusal its envelope names.
pub(crate) fn answer(route: &str, response: &mut Response) -> Result<Value, Error> {
    let status = response.status().as_u16();
    let text = body_text(response, route)?;
    if !(200..300).contains(&status) {
        return Err(Error::refusal(route, status, &text));
    }
    serde_json::from_str(&text).map_err(|e| Error::unreadable(route, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_never_leaves_its_path_segment() {
        assert_eq!(segment("local_19a2f.c:1-x"), Ok("local_19a2f.c:1-x"));
        for bad in ["", ".", "..", "../threads", "a/b", "a?b", "a%2Fb", "é"] {
            assert_eq!(segment(bad), Err(Error::InvalidId(bad.to_owned())), "{bad}");
        }
    }

    #[test]
    fn keys_differ_from_call_to_call() {
        assert_ne!(idempotency_key(), idempotency_key());
    }
}
