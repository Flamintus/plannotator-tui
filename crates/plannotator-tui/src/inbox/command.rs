//! `plannotator-tui inbox [THREAD] [--sent]`: the Inbox's threads, one per line, or one
//! thread's agent messages and attached files to review.

#![allow(clippy::print_stdout, reason = "the thread list is the command's output")]

use std::path::PathBuf;

use anyhow::Result;

use crate::app::App;
use crate::cli;
use crate::config::{Config, config_path};
use crate::delivery::Clipboard;

const USAGE: &str = "usage: plannotator-tui inbox [<thread-id> [--sent]]";

pub(crate) fn run(args: &[String]) -> Result<()> {
    let mut thread = None;
    let mut sent = false;
    for arg in args {
        match arg.as_str() {
            "--sent" => sent = true,
            flag if flag.starts_with('-') => anyhow::bail!("unknown flag {flag}\n{USAGE}"),
            id if thread.is_none() => thread = Some(id.to_owned()),
            extra => anyhow::bail!("unexpected argument {extra:?}\n{USAGE}"),
        }
    }
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let path = config_path(|key| std::env::var(key).ok(), &home);
    anyhow::ensure!(
        Config::load_from(&path)?.inbox.enabled,
        "the Inbox is off: set `enabled = true` under `[inbox]` in {}",
        path.display()
    );
    let inbox = super::connect()?;
    let Some(thread) = thread else {
        for row in inbox.threads()? {
            let name = row.thread_name.or(row.subject).unwrap_or_default();
            println!("{}  {:<8} {name}", row.thread_id, row.section);
        }
        return Ok(());
    };
    let found = super::source::thread_documents(inbox.as_ref(), &thread, sent)?;
    // The thread's session is the Inbox's to reach: a document whose notes it cannot keep
    // copies its feedback, never pasting it into whatever agent pane Herdr has in focus.
    cli::run_ui(|width| App::open_inbox_thread(found, inbox, width, Box::new(Clipboard)))
}
