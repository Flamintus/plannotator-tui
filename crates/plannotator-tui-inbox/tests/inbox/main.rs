//! The client against a stand-in Inbox (`support`) and the contract fixtures
//! (`tests/fixtures`, see the crate's README for where they come from).

mod contract;
mod door;
mod support;
mod watch;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
