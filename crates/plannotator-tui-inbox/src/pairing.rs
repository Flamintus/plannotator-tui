//! Local pairing and the device token.
//!
//! Contract E7 (decision 2026-11-05 01:03:15): a terminal pairs with no gesture, through the
//! loopback port only, as a device of platform `terminal`, by a secret the Inbox writes and
//! the client reads. This client presents the registry's connection token (`inbox.json`,
//! 0600, the same account's only) to `POST pair-local` and keeps the `tok_…` device token
//! it gets back in a file of its own, 0600, under the data directory. A revoked token is
//! answered `401 device_revoked`: forget the file and pair again (`crate::connect` does).
//!
//! Same limit as every pairing (analysis § 5.4): an agent under the same account reads
//! these files too. The door bounds, attributes and revokes; it does not prove a person.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::discovery::Discovery;
use crate::error::Error;
use crate::http::{self, CALL_TIMEOUT, DOOR_PREFIX};

/// The platform a terminal pairs as.
pub const PLATFORM: &str = "terminal";

/// A device's bearer token. Never printed: `Debug` shows only that there is one.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Token(String);

impl Token {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    pub(crate) fn secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(…)")
    }
}

/// A paired device as the door answers it (`publicDevice`: never the token's hash).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// What local pairing gives, and what the token file keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paired {
    pub token: Token,
    pub device: Device,
}

#[derive(Serialize, Deserialize)]
struct TokenFile {
    v: u64,
    #[serde(flatten)]
    paired: Paired,
}

/// Where plannotator-tui keeps its device token: beside its other files in the data directory.
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("clients").join("plannotator-tui").join("inbox-device.json")
}

/// Pair this terminal with the Inbox found, as `name`.
pub fn pair_local(found: &Discovery, name: &str) -> Result<Paired, Error> {
    let route = "pair-local";
    let body = json!({ "name": name, "platform": PLATFORM, "idempotency_key": http::idempotency_key() });
    let mut response = http::agent(CALL_TIMEOUT)
        .post(format!("{}{DOOR_PREFIX}{route}", found.base_url))
        .header("Authorization", format!("Bearer {}", found.connection_token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| Error::transport(route, e))?;
    let value = http::answer(route, &mut response)?;
    serde_json::from_value(value).map_err(|e| Error::unreadable(route, e))
}

impl Paired {
    /// The pairing kept at `path`, or `None` when there is none yet.
    pub fn load(path: &Path) -> Result<Option<Self>, Error> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error(path, e)),
        };
        let file: TokenFile = serde_json::from_str(&text).map_err(|e| io_error(path, e))?;
        match file.v {
            1 => Ok(Some(file.paired)),
            v if v > 1 => Err(Error::Newer { what: path.display().to_string(), v }),
            _ => Err(io_error(path, "not a device token file")),
        }
    }

    /// Keep the pairing at `path`, owner-only, by a temporary file and a rename.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let dir = path.parent().ok_or_else(|| io_error(path, "no parent directory"))?;
        create_private_dir(dir).map_err(|e| io_error(dir, e))?;
        let text = serde_json::to_string_pretty(&TokenFile { v: 1, paired: self.clone() })
            .map_err(|e| io_error(path, e))?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let write = || -> std::io::Result<()> {
            let mut file = private_file(&temp)?;
            file.write_all(text.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            std::fs::rename(&temp, path)
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            io_error(path, e)
        })
    }

    /// Forget the pairing kept at `path` (a revoked token); nothing kept is no error.
    pub fn forget(path: &Path) -> Result<(), Error> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_error(path, e)),
            _ => Ok(()),
        }
    }
}

fn io_error(path: &Path, detail: impl fmt::Display) -> Error {
    Error::Io { path: path.display().to_string(), detail: detail.to_string() }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_never_printed() {
        let paired = Paired {
            token: Token::new("tok_secret"),
            device: Device {
                id: "dev_1".into(),
                name: "t".into(),
                platform: PLATFORM.into(),
                other: Map::new(),
            },
        };
        assert!(!format!("{paired:?}").contains("tok_secret"));
    }
}
