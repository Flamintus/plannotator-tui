//! The one error type of the crate, read from the CLI's own exit codes and words.
//!
//! `plannotator threads` exits 0 with one JSON value on stdout, 1 when the journal refuses
//! (`<code>: <message>` on stderr: `not_found`, `invalid`, `conflict`, `locked`, or `empty`
//! for a round with nothing to send), 2 on a call it does not read. The kinds a caller acts on differently get their own variant; the rest
//! keep the CLI's words.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The program could not be started: not there, or not runnable.
    Spawn { program: String, detail: String },
    /// It did not answer in time, and was stopped.
    Timeout { action: String },
    /// The journal refused the call (exit 1), in its words.
    Refused { action: String, code: String, message: String },
    /// The CLI does not read this call (exit 2): a Plannotator without the review journal,
    /// or a call this client got wrong.
    Usage { action: String, detail: String },
    /// It exited otherwise, or was stopped by a signal.
    Failed { action: String, detail: String },
    /// The answer is in a contract newer than this client reads (`v` above [`crate::CONTRACT`]).
    Newer { what: String, v: u64 },
    /// The answer is not in the shape this client reads.
    Unreadable { action: String, detail: String },
    /// A call refused before the CLI runs: a `now` send that names no draft.
    Invalid(String),
}

impl Error {
    /// The journal's own code, when it refused.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Refused { code, .. } => Some(code),
            _ => None,
        }
    }

    /// A refusal from what the CLI wrote on stderr: its first `<code>: <message>` line.
    pub(crate) fn refusal(action: &str, stderr: &str) -> Self {
        let line = stderr.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().trim();
        let (code, message) = match line.split_once(": ") {
            Some((code, message))
                if !code.is_empty() && code.chars().all(|c| c.is_ascii_lowercase() || c == '_') =>
            {
                (code.to_owned(), message.to_owned())
            }
            _ => (String::new(), line.to_owned()),
        };
        Self::Refused { action: action.to_owned(), code, message }
    }

    pub(crate) fn unreadable(action: &str, detail: impl fmt::Display) -> Self {
        Self::Unreadable { action: action.to_owned(), detail: detail.to_string() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, detail } => write!(f, "could not run {program}: {detail}"),
            Self::Timeout { action } => write!(f, "threads {action}: no answer in time, stopped"),
            Self::Refused { action, code, message } if code.is_empty() => {
                write!(f, "threads {action}: {message}")
            }
            Self::Refused { action, code, message } => write!(f, "threads {action}: {message} ({code})"),
            Self::Usage { action, detail } => {
                write!(f, "threads {action} is not a call this Plannotator reads: {detail}")
            }
            Self::Failed { action, detail } => write!(f, "threads {action} failed: {detail}"),
            Self::Newer { what, v } => {
                write!(f, "{what} is in contract v{v}, newer than this client (v{})", crate::CONTRACT)
            }
            Self::Unreadable { action, detail } => write!(f, "threads {action}: unexpected answer: {detail}"),
            Self::Invalid(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_keeps_the_journals_code_and_words() {
        let err = Error::refusal("resolve", "not_found: No sent comment or sub-thread nope.\n");
        assert_eq!(err.code(), Some("not_found"));
        assert_eq!(err.to_string(), "threads resolve: No sent comment or sub-thread nope. (not_found)");
        let err = Error::refusal("comment", "\ninvalid: id: letters, digits\n");
        assert_eq!(err.code(), Some("invalid"));
        assert_eq!(err.to_string(), "threads comment: id: letters, digits (invalid)");
        let err = Error::refusal("comment", "Something broke.");
        assert_eq!(err.code(), Some(""), "a line with no code keeps its words whole");
        assert_eq!(err.to_string(), "threads comment: Something broke.");
    }
}
