//! The one error type of the crate, read from the Inbox's own envelope.
//!
//! The door and the window routes answer a refusal as `{ error, code, …details }` with an
//! HTTP status (`packages/server/inbox-devices.ts`, `refuse`; `InboxError` in the window
//! routes). The kinds a caller acts on differently get their own variant: pair again, the
//! route is missing (the door is closed to this client, or the Inbox is older than it), a
//! setting is off. Everything else keeps the Inbox's words.

use std::fmt;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No Inbox answers for this data directory: never started, stopped, or the port in
    /// `inbox.json` is held by something else. The reason says which.
    NotRunning(String),
    /// The door refused this terminal's token (a `401`: `device_token_invalid`,
    /// `device_revoked`, …): pair again. Local pairing's own refusal is a [`Error::Refused`].
    Unpaired { code: String },
    /// The door has no such route (`404 device_route_not_found`): it is closed to this
    /// client, or the Inbox predates the route.
    RouteMissing { route: String },
    /// The route needs a setting that is off (`409 persistent_comments_off`).
    SettingOff { route: String, code: String },
    /// Any other refusal, in the Inbox's words.
    Refused { route: String, status: u16, code: String, message: String, field: Option<String> },
    /// The answer is in a contract newer than this client reads (`v` above [`crate::CONTRACT`]).
    Newer { what: String, v: u64 },
    /// The answer is not in the shape this client reads.
    Unreadable { route: String, detail: String },
    /// The request did not complete: refused, reset or timed out.
    Transport { route: String, detail: String },
    /// Reading or writing a local file (the device token).
    Io { path: String, detail: String },
    /// A call refused before any request: an id that is not one the Inbox makes, a `now`
    /// send that names no draft.
    Invalid(String),
}

impl Error {
    /// The Inbox's own code, when the error carries one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Unpaired { code } | Self::SettingOff { code, .. } | Self::Refused { code, .. } => {
                Some(code)
            }
            _ => None,
        }
    }

    /// The error from a refused answer: its status and body as the Inbox sent them.
    pub(crate) fn refusal(route: &str, status: u16, body: &str) -> Self {
        let envelope: Envelope = serde_json::from_str(body).unwrap_or_default();
        let code = envelope.code.unwrap_or_default();
        match (status, code.as_str()) {
            (401, _) => Self::Unpaired { code },
            // The door's unknown route, and the window's unknown `/api/` path behind it.
            (404, "device_route_not_found" | "not_found" | "") => {
                Self::RouteMissing { route: route.to_owned() }
            }
            (409, "persistent_comments_off") => Self::SettingOff { route: route.to_owned(), code },
            _ => Self::Refused {
                route: route.to_owned(),
                status,
                code,
                message: envelope.error.unwrap_or_default(),
                field: envelope.field,
            },
        }
    }

    /// The error from a refused local pairing: a `401` there refuses the connection token
    /// presented, so pairing again would only be refused again.
    pub(crate) fn pairing_refusal(route: &str, status: u16, body: &str) -> Self {
        match Self::refusal(route, status, body) {
            Self::Unpaired { code } => {
                let envelope: Envelope = serde_json::from_str(body).unwrap_or_default();
                let message = envelope.error.unwrap_or_default();
                Self::Refused { route: route.to_owned(), status, code, message, field: envelope.field }
            }
            other => other,
        }
    }

    pub(crate) fn unreadable(route: &str, detail: impl fmt::Display) -> Self {
        Self::Unreadable { route: route.to_owned(), detail: detail.to_string() }
    }

    pub(crate) fn transport(route: &str, detail: impl fmt::Display) -> Self {
        Self::Transport { route: route.to_owned(), detail: detail.to_string() }
    }
}

#[derive(Debug, Default, Deserialize)]
struct Envelope {
    error: Option<String>,
    code: Option<String>,
    field: Option<String>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning(reason) => write!(f, "the Plannotator Inbox is not running: {reason}"),
            Self::Unpaired { code } => {
                write!(f, "the Inbox does not know this terminal ({code}): pair again")
            }
            Self::RouteMissing { route } => {
                write!(f, "the Inbox has no route {route} for this terminal (door closed, or an older Inbox)")
            }
            Self::SettingOff { route, code } => write!(f, "{route}: a setting of the Inbox is off ({code})"),
            Self::Refused { route, status, code, message, field } => {
                write!(f, "{route}: {message} ({status} {code}")?;
                if let Some(field) = field {
                    write!(f, ", field {field}")?;
                }
                f.write_str(")")
            }
            Self::Newer { what, v } => {
                write!(f, "{what} is in contract v{v}, newer than this client (v{})", crate::CONTRACT)
            }
            Self::Unreadable { route, detail } => write!(f, "{route}: unexpected answer: {detail}"),
            Self::Transport { route, detail } => write!(f, "{route}: {detail}"),
            Self::Io { path, detail } => write!(f, "{path}: {detail}"),
            Self::Invalid(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_sorted_by_what_the_caller_does_next() {
        let body = |code: &str| format!(r#"{{"error":"words","code":"{code}"}}"#);
        assert_eq!(
            Error::refusal("health", 401, &body("device_revoked")),
            Error::Unpaired { code: "device_revoked".into() }
        );
        assert_eq!(
            Error::refusal("threads/t/comments", 404, &body("device_route_not_found")),
            Error::RouteMissing { route: "threads/t/comments".into() }
        );
        // An unknown `/api/` path behind the door, and a body that is not JSON at all.
        assert!(matches!(Error::refusal("x", 404, &body("not_found")), Error::RouteMissing { .. }));
        assert!(matches!(Error::refusal("x", 404, "Not found"), Error::RouteMissing { .. }));
        assert!(matches!(
            Error::refusal("x", 409, &body("persistent_comments_off")),
            Error::SettingOff { .. }
        ));
        // Local pairing refused is no reason to pair again.
        assert!(matches!(
            Error::pairing_refusal("pair-local", 401, &body("device_token_invalid")),
            Error::Refused { status: 401, .. }
        ));
        // A 404 that names a missing thing is a refusal, not a missing route.
        assert_eq!(Error::refusal("x", 404, &body("thread_not_found")).code(), Some("thread_not_found"));
        assert_eq!(
            Error::refusal(
                "x",
                422,
                r#"{"error":"bad","code":"validation_error","field":"annotation.text"}"#
            ),
            Error::Refused {
                route: "x".into(),
                status: 422,
                code: "validation_error".into(),
                message: "bad".into(),
                field: Some("annotation.text".into()),
            }
        );
    }
}
