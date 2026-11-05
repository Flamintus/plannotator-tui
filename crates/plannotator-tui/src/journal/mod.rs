//! The review loop: `last`'s replies, and a file or a folder sent to an agent in Herdr,
//! reviewed round after round through Plannotator's review journal, which keeps the comments,
//! the agent's replies under them by id, sub-threads and resolutions, by host and session
//! for `last`, by real path for a file or a folder. The one seam is
//! `plannotator_tui_journal::Journal`, whose `Cli` runs `plannotator threads` and whose tests
//! put a stand-in in its place.
//!
//! Off unless `PLANNOTATOR_TUI_REVIEW_LOOP` turns it on (the config file stays the one the
//! released binary reads) and a `plannotator` with the journal's commands is on the `PATH`;
//! off, nothing here runs and every review is what it always was.
//!
//! - `passage`: a comment's quote found in a document's words as drawn.
//! - `tree`: what is in the view of a review's threads, and a comment's thread as a document.

pub(crate) mod passage;
#[cfg(test)]
pub(crate) mod stand_in;
pub(crate) mod tree;

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result};
use plannotator_tui_journal::{Anchor, Cli, Journal, ReviewTarget, SendTo};

/// The journal, shared by every document of one review.
pub(crate) type Shared = Rc<dyn Journal>;

/// The variable that turns the review loop on: `1` or `true`.
pub(crate) const SWITCH: &str = "PLANNOTATOR_TUI_REVIEW_LOOP";

/// Whether `env` turns the review loop on.
pub(crate) fn switched_on(env: impl Fn(&str) -> Option<String>) -> bool {
    env(SWITCH).is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

/// A review of the loop: `last`'s replies, through the journal of `host`'s session
/// `session`, or a file or a folder, its sends to the agent in Herdr.
#[derive(Clone)]
pub(crate) struct Review {
    pub(crate) journal: Shared,
    /// The session's review (`last` with Plannotator's name of the host), or the file's or
    /// the folder's.
    pub(crate) target: ReviewTarget,
    /// The agent session a send goes to, when the target names none: a file's or a folder's.
    pub(crate) to: Option<SendTo>,
}

impl std::fmt::Debug for Review {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Review").field("target", &self.target).field("to", &self.to).finish_non_exhaustive()
    }
}

impl Review {
    /// The review of `host`'s session `session`, `host` as this app names it (`claude`).
    pub(crate) fn of(journal: Shared, host: &str, session: &str) -> Self {
        let host = crate::archive::origin_label(host).to_owned();
        Self { journal, target: ReviewTarget::Last { host, session: session.to_owned() }, to: None }
    }

    /// The review of file or folder `path`, sent to `to`.
    pub(crate) fn of_path(journal: Shared, path: &Path, to: SendTo) -> Self {
        let path = path.display().to_string();
        Self { journal, target: ReviewTarget::File { path }, to: Some(to) }
    }

    /// The agent session the review's sends go to.
    pub(crate) fn session(&self) -> Option<&str> {
        match (&self.to, &self.target) {
            (Some(to), _) => Some(&to.session),
            (None, ReviewTarget::Last { session, .. }) => Some(session),
            (None, ReviewTarget::File { .. }) => None,
        }
    }
}

/// Plannotator's CLI from the `PATH`, once it showed it has the journal's actions.
fn cli() -> Result<Shared> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let program = Cli::find(&path).context("no `plannotator` on the PATH")?;
    let cli = Cli::new(program).probe().context("plannotator")?;
    Ok(Rc::new(cli))
}

/// The review loop for `last`: `Ok(None)` when it is off; an error says why it cannot run
/// although it is on. `session` is the host's own id for the session, when it was found
/// exactly: a folder's newest transcript is a guess, and gets no review of its own.
pub(crate) fn connect(session: Option<&str>) -> Result<Option<Shared>> {
    if !switched_on(|key| std::env::var(key).ok()) {
        return Ok(None);
    }
    session.context("this agent's session is not identified exactly, so it has no review of its own")?;
    cli().map(Some)
}

/// The review loop for a file or a folder: `Ok(None)` when it is off; an error says why it
/// cannot run although it is on. `agent` is asked only then: the agent the review sends to,
/// its host as this app names it (`claude`) and the host's own id for its session, found
/// exactly; with none, a round would have no session to go to.
pub(crate) fn connect_path(
    agent: impl FnOnce() -> Option<(String, String)>,
) -> Result<Option<(Shared, SendTo)>> {
    if !switched_on(|key| std::env::var(key).ok()) {
        return Ok(None);
    }
    let (host, session) = agent().context("no agent in Herdr whose session is identified exactly")?;
    let to = SendTo { host: Some(crate::archive::origin_label(&host).to_owned()), session };
    Ok(Some((cli()?, to)))
}

/// A file as the journal's comments name it: its real path, else as given.
pub(crate) fn real_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `on`, a comment's place, is file `path` (a real path): Plannotator's screens may
/// name it by another path to the same file.
pub(crate) fn on_file(on: &Anchor, path: &Path) -> bool {
    let Anchor::File { path: on, .. } = on else { return false };
    Path::new(on) == path || real_path(Path::new(on)) == path
}

/// What a document of the review loop is: the words its comments are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// An agent's message, by the id its host gave it.
    Message { message_id: String },
    /// A file of a file's or a folder's review, by its real path.
    File { path: PathBuf },
    /// A comment's thread, as `tree::document` writes it: its comments are the sub-threads
    /// on the words of its messages, each message's words at its source range.
    Thread { comment_id: String, messages: Vec<(String, Range<usize>)> },
}

/// A passage of a sent message of a thread (a comment, a reply or a sub-thread) as drawn in
/// its box: what a sub-thread is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Passage {
    /// The message's id.
    pub(crate) parent: String,
    /// The words, as the message has them.
    pub(crate) quote: String,
    /// Where they start in the message's words, as Plannotator counts it
    /// (`passage::offset_in`); `None`: the journal finds them.
    pub(crate) offset: Option<u64>,
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
