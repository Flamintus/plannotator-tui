//! The Plannotator Inbox, as this app reviews through it. Plannotator's local server keeps
//! threads between a person and coding agents; this app opens their messages and attached
//! files as documents. The one seam is `plannotator_tui_journal::Inbox`, whose `Client`
//! speaks the Inbox's device door and whose tests put a stand-in in its place.
//!
//! All of it waits behind `[inbox] enabled`, off by default: off, nothing here runs.
//!
//! - `command`: `plannotator-tui inbox`, the threads or one thread to review.
//! - `passage`: a comment's quote found in a document's words as drawn.
//! - `source`: a thread's agent messages and attached text files, as documents.
//! - `tree`: what is in the Inbox's view of a thread's comments, and a comment's thread
//!   as a document.

pub(crate) mod command;
pub(crate) mod passage;
pub(crate) mod source;
#[cfg(test)]
pub(crate) mod stand_in;
pub(crate) mod tree;

use std::ops::Range;
use std::rc::Rc;

use anyhow::{Context, Result};
use plannotator_tui_journal::Inbox;
use plannotator_tui_schema::{DocumentSource, Provenance};
use serde::{Deserialize, Serialize};

/// `[inbox]` in the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct InboxConfig {
    /// Review through the Plannotator Inbox. Off by default.
    pub(crate) enabled: bool,
}

/// The Inbox, shared by every document of one review.
pub(crate) type Shared = Rc<dyn Inbox>;

/// The name this terminal pairs with the Inbox under.
const DEVICE_NAME: &str = "plannotator-tui";

/// The Inbox under this process's data directory, this terminal paired with it.
pub(crate) fn connect() -> Result<Shared> {
    let data_dir = crate::workspace_paths::data_dir();
    let client = plannotator_tui_journal::connect(&data_dir, DEVICE_NAME)
        .with_context(|| format!("connecting to the Inbox under {}", data_dir.display()))?;
    Ok(Rc::new(client))
}

/// What an Inbox document is: the words its comments are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// An agent's message.
    Message { thread_id: String, message_id: String },
    /// A text file attached to an agent's message, at the version its comments name.
    Attachment {
        thread_id: String,
        message_id: String,
        attachment_id: String,
        /// The file's path, as the thread's comments name it.
        path: String,
        /// `current`, or the sent version's sha256, as the Inbox answered it.
        version: String,
        /// The version the agent sent, rather than the file as it is now.
        sent: bool,
    },
    /// A comment's thread, as `tree::document` writes it: its comments are the sub-threads
    /// on the words of its messages, each message's words at its source range.
    Thread { thread_id: String, comment_id: String, messages: Vec<(String, Range<usize>)> },
}

impl Target {
    pub(crate) fn thread_id(&self) -> &str {
        match self {
            Self::Message { thread_id, .. }
            | Self::Attachment { thread_id, .. }
            | Self::Thread { thread_id, .. } => thread_id,
        }
    }

    /// The document of these words: transient, since the Inbox keeps what is said about it.
    pub(crate) fn source(&self, name: String, text: String) -> DocumentSource {
        DocumentSource::new(text, name, true, self.provenance())
    }

    pub(crate) fn provenance(&self) -> Provenance {
        match self {
            Self::Message { thread_id, message_id } => Provenance::Inbox {
                thread_id: thread_id.clone(),
                message_id: message_id.clone(),
                attachment_id: None,
            },
            Self::Attachment { thread_id, message_id, attachment_id, .. } => Provenance::Inbox {
                thread_id: thread_id.clone(),
                message_id: message_id.clone(),
                attachment_id: Some(attachment_id.clone()),
            },
            Self::Thread { thread_id, comment_id, .. } => Provenance::Inbox {
                thread_id: thread_id.clone(),
                message_id: comment_id.clone(),
                attachment_id: None,
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use crate::config::Config;

    #[test]
    fn the_inbox_is_off_unless_the_config_turns_it_on() {
        assert!(!Config::default().inbox.enabled);
        assert!(!Config::parse("").expect("parses").inbox.enabled);
        assert!(Config::parse("[inbox]\nenabled = true\n").expect("parses").inbox.enabled);
        let err = Config::parse("[inbox]\nenable = true\n").expect_err("a typo is an error");
        assert!(err.to_string().contains("enable"), "{err}");
    }
}
