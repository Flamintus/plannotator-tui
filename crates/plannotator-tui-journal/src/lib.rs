//! A client of Plannotator's review journal, for terminal clients.
//!
//! Plannotator keeps each review's comment threads in a journal on disk, by target
//! (`reviews/<key>/journal.jsonl` under its data directory): the person's comments, replies
//! and sub-threads, their resolutions, the sends that carry them to the agent's session, and
//! the agent's replies under them by id. Its CLI, `plannotator threads`, reads and writes the
//! journal for a client with no server of its own; the agent answers through the same CLI
//! (`plannotator reply`). This crate runs it.
//!
//! - [`Journal`] is the seam: [`Cli`] runs `plannotator threads <action> -` with the call's
//!   JSON on stdin; an application's tests put a stand-in.
//! - The thread view ([`ThreadView`], `v` [`CONTRACT`]) is computed by Plannotator: rounds,
//!   states, folds, statuses, where each sub-thread sits. A client draws it and applies none
//!   of those rules itself.
//! - [`Journal::stamp`] tells when a review's journal was last written without running the
//!   CLI, so a client reads the view again only when it moved.
//!
//! Synchronous, no runtime: every call runs the CLI once, and stops it if it does not answer
//! in time.

mod cli;
mod error;
mod records;
mod requests;
mod view;

pub use cli::{CONTRACT, Cli, Journal, Stamp};
pub use error::Error;
pub use records::{Send, SendTo, Sent, Viewed};
pub use requests::{IdempotencyKey, NewComment, Placement, ReviewTarget, SendMode};
pub use view::{Anchor, Author, Parent, Place, ReplyNode, State, Status, ThreadNode, ThreadView};
