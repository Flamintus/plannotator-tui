//! Finding the Inbox: `inbox/inbox.json` under Plannotator's data directory, then its
//! health answer on the loopback port that file names.
//!
//! The file (`packages/shared/inbox/registry.ts`) is `{ v: 1, pid, port, url, version,
//! token, serverSession, startedAt, … }`, mode 0600, written when the Inbox starts and left
//! in place when it stops. The Inbox's rule for callers is that it is read on every call,
//! never cached: a restart may move the port. The data directory itself is the caller's
//! (`plannotator_tui_schema::data_dir`, which honours `PLANNOTATOR_DATA_DIR`).

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::error::Error;
use crate::http;

/// How long the health check waits, as Plannotator's own probe does.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);

/// The `app` a health answer names when it comes from an Inbox.
const INBOX_APP_ID: &str = "plannotator-inbox";

/// A running Inbox, as found.
#[derive(Clone)]
pub struct Discovery {
    pub data_dir: PathBuf,
    /// `http://127.0.0.1:<port>`.
    pub base_url: String,
    pub server_session: String,
    /// The running Inbox's version (`dev` for a source run).
    pub version: String,
    /// The registry's connection token: what local pairing presents. Never printed.
    pub(crate) connection_token: String,
}

impl fmt::Debug for Discovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Discovery")
            .field("data_dir", &self.data_dir)
            .field("base_url", &self.base_url)
            .field("server_session", &self.server_session)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// The registry entry, as far as this client reads it.
#[derive(Deserialize)]
pub(crate) struct Registry {
    v: u64,
    pub(crate) port: u16,
    pub(crate) token: String,
    #[serde(rename = "serverSession")]
    pub(crate) server_session: String,
}

impl Registry {
    pub(crate) fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

pub(crate) fn registry_path(data_dir: &Path) -> PathBuf {
    data_dir.join("inbox").join("inbox.json")
}

/// The registry entry under `data_dir`, read now.
pub(crate) fn read_registry(data_dir: &Path) -> Result<Registry, Error> {
    let path = registry_path(data_dir);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::NotRunning(format!("no Inbox registry at {} ({e})", path.display())))?;
    let entry: Registry = serde_json::from_str(&text)
        .map_err(|e| Error::NotRunning(format!("{} is not an Inbox registry ({e})", path.display())))?;
    match entry.v {
        1 => Ok(entry),
        v if v > 1 => Err(Error::Newer { what: path.display().to_string(), v }),
        _ => Err(Error::NotRunning(format!("{} is not an Inbox registry", path.display()))),
    }
}

#[derive(Deserialize)]
pub(crate) struct WindowHealth {
    ok: bool,
    app: String,
    version: String,
    #[serde(rename = "serverSession")]
    server_session: String,
}

/// The registry under `data_dir`, once the health answer on its port (no token sent)
/// names its `serverSession`: the Inbox's own test of a running Inbox. The registry stays
/// in place when the Inbox stops, so a token only goes to a port that passed it.
pub(crate) fn running(data_dir: &Path) -> Result<(Registry, WindowHealth), Error> {
    let registry = read_registry(data_dir)?;
    let base_url = registry.base_url();
    let not_running = |why: &str| Error::NotRunning(format!("{base_url}: {why}"));
    let mut response = http::agent(HEALTH_TIMEOUT)
        .get(format!("{base_url}/api/inbox/health"))
        .call()
        .map_err(|e| not_running(&format!("no answer ({e})")))?;
    let status = response.status().as_u16();
    let body = http::body_text(&mut response, "health")?;
    if status != 200 {
        return Err(not_running(&format!("health answered {status}")));
    }
    let health: WindowHealth =
        serde_json::from_str(&body).map_err(|_| not_running("not an Inbox's answer"))?;
    if !health.ok || health.app != INBOX_APP_ID {
        return Err(not_running("not an Inbox's answer"));
    }
    if health.server_session != registry.server_session {
        return Err(not_running("another server answers on the registry's port"));
    }
    Ok((registry, health))
}

/// The Inbox for `data_dir`, when one runs: the registry, then the health answer on its
/// port, which must name the registry's `serverSession` (another one means the port was
/// taken by a newer Inbox or something else since).
pub fn discover(data_dir: &Path) -> Result<Discovery, Error> {
    let (registry, health) = running(data_dir)?;
    Ok(Discovery {
        data_dir: data_dir.to_path_buf(),
        base_url: registry.base_url(),
        server_session: health.server_session,
        version: health.version,
        connection_token: registry.token,
    })
}
