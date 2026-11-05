//! The review loop: `last`'s replies reviewed round after round through Plannotator's review
//! journal, which keeps the comments, the agent's replies under them by id, sub-threads and
//! resolutions, by host and session. The one seam is `plannotator_tui_journal::Journal`,
//! whose `Cli` runs `plannotator threads` and whose tests put a stand-in in its place.
//!
//! Off unless `PLANNOTATOR_TUI_REVIEW_LOOP` turns it on (the config file stays the one the
//! released binary reads) and a `plannotator` with the journal's commands is on the `PATH`;
//! off, nothing here runs and `last` is what it always was.
//!
//! - `passage`: a comment's quote found in a document's words as drawn.
//! - `tree`: what is in the view of a review's threads, and a comment's thread as a document.

pub(crate) mod passage;
#[cfg(test)]
pub(crate) mod stand_in;
pub(crate) mod tree;

use std::ops::Range;
use std::rc::Rc;

use anyhow::{Context, Result};
use plannotator_tui_journal::{Cli, Journal, ReviewTarget};

/// The journal, shared by every document of one review.
pub(crate) type Shared = Rc<dyn Journal>;

/// The variable that turns the review loop on: `1` or `true`.
pub(crate) const SWITCH: &str = "PLANNOTATOR_TUI_REVIEW_LOOP";

/// Whether `env` turns the review loop on.
pub(crate) fn switched_on(env: impl Fn(&str) -> Option<String>) -> bool {
    env(SWITCH).is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

/// The review of `last`'s replies, through the journal of `host`'s session `session`.
#[derive(Clone)]
pub(crate) struct Review {
    pub(crate) journal: Shared,
    /// The session's review: `last` with Plannotator's name of the host.
    pub(crate) target: ReviewTarget,
}

impl std::fmt::Debug for Review {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Review").field("target", &self.target).finish_non_exhaustive()
    }
}

impl Review {
    /// The review of `host`'s session `session`, `host` as this app names it (`claude`).
    pub(crate) fn of(journal: Shared, host: &str, session: &str) -> Self {
        let host = crate::archive::origin_label(host).to_owned();
        Self { journal, target: ReviewTarget::Last { host, session: session.to_owned() } }
    }
}

/// The review loop for `last`: `Ok(None)` when it is off; an error says why it cannot run
/// although it is on. `session` is the host's own id for the session, when it was found
/// exactly: a folder's newest transcript is a guess, and gets no review of its own.
pub(crate) fn connect(session: Option<&str>) -> Result<Option<Shared>> {
    if !switched_on(|key| std::env::var(key).ok()) {
        return Ok(None);
    }
    session.context("this agent's session is not identified exactly, so it has no review of its own")?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let program = Cli::find(&path).context("no `plannotator` on the PATH")?;
    let cli = Cli::new(program).probe().context("plannotator")?;
    Ok(Some(Rc::new(cli)))
}

/// What a document of the review loop is: the words its comments are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// An agent's message, by the id its host gave it.
    Message { message_id: String },
    /// A comment's thread, as `tree::document` writes it: its comments are the sub-threads
    /// on the words of its messages, each message's words at its source range.
    Thread { comment_id: String, messages: Vec<(String, Range<usize>)> },
}

#[cfg(test)]
mod tests {
    use super::switched_on;

    #[test]
    fn only_one_or_true_turns_the_review_loop_on() {
        let env = |value: &'static str| move |key: &str| (key == super::SWITCH).then(|| value.to_owned());
        assert!(switched_on(env("1")));
        assert!(switched_on(env(" TRUE ")));
        assert!(!switched_on(env("0")));
        assert!(!switched_on(env("")));
        assert!(!switched_on(|_| None));
    }
}
