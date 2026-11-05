//! A client of the Plannotator Inbox's person API, for terminal clients.
//!
//! The Inbox (Plannotator's local server) keeps threads between a person and coding agents;
//! its device door (`/api/inbox/device/*`) is the API every client that is not its own
//! window uses. This crate speaks it from a terminal: it finds the Inbox, pairs locally,
//! reads threads and the computed view of their comment threads, writes comments, replies,
//! sub-threads, resolutions and sends, and follows the event stream.
//!
//! - [`discover`] reads `inbox/inbox.json` under the data directory and checks the Inbox's
//!   health; [`connect`] also pairs (or reuses the kept token) and gives a [`Client`].
//! - [`Inbox`] is the seam: [`Client`] speaks HTTP, an application's tests put a stand-in.
//! - [`Inbox::watch`] gives a [`Watch`]: threads of its own, events on a channel.
//!
//! Synchronous, no runtime. The contract it reads is E7's (`v` [`CONTRACT`]); routes E7
//! has not put on the door yet answer [`Error::RouteMissing`].

mod client;
mod discovery;
mod error;
mod http;
mod pairing;
mod records;
mod requests;
mod sse;
mod view;
mod watch;

pub use client::{CONTRACT, Client, Inbox, connect};
pub use discovery::{Discovery, discover};
pub use error::Error;
pub use pairing::{Device, PLATFORM, Paired, Token, pair_local, token_path};
pub use records::{
    AttachmentText, CommentRecord, CommentSend, Deposit, Health, LastThread, Message, ProjectSessions,
    SendTarget, Session, SessionProject, Thread, ThreadRow,
};
pub use requests::{
    AttachmentVersion, NewComment, NewReply, NewSubThread, Passage, ReviewTarget, SendMode, Target,
};
pub use view::{
    Anchor, Author, FileVersion, Parent, Place, ReplyNode, Side, State, Status, ThreadNode, ThreadView,
};
pub use watch::{Event, GATHER, Kind, Record, Watch};
