//! The reviewed reply of `last`, deposited in the Inbox: the agent's chat message becomes a
//! message of its session's review thread (one per session and target, P1's operation,
//! shared with `/plannotator-last`), where its comments are kept and sent.
//!
//! Only for a session the Inbox knows, by its host's own session id: one Herdr did not
//! identify is never guessed from the folder (analysis § 4.5). Anything else leaves `last`
//! as it was, its feedback sent to the agent's pane.

use anyhow::{Context, Result};
use plannotator_tui_inbox::{Deposit, ReviewTarget};

use super::Shared;

/// Where `last`'s replies go in the Inbox: their session, as the Inbox knows it.
pub(crate) struct Depositor {
    inbox: Shared,
    /// The Inbox's name of the session's host (`claude-code`), and the host's session id.
    host: String,
    session: String,
    /// The session's project folder.
    project_path: String,
}

impl std::fmt::Debug for Depositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Depositor")
            .field("host", &self.host)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// `last`'s depositor: `None` with the Inbox off; an error says why replies cannot go there.
pub(crate) fn depositor(enabled: bool, session_id: Option<&str>) -> Result<Option<Depositor>> {
    if !enabled {
        return Ok(None);
    }
    let session_id = session_id
        .context("this agent's session is not identified exactly, so it has no Inbox session to review in")?;
    Depositor::find(super::connect()?, session_id).map(Some)
}

impl Depositor {
    /// The live session `session_id` of the Inbox, by its host's id.
    pub(crate) fn find(inbox: Shared, session_id: &str) -> Result<Self> {
        let projects = inbox.sessions().context("listing the Inbox's sessions")?;
        let found = projects.iter().find_map(|project| {
            let session = project.sessions.iter().find(|session| session.session == session_id)?;
            Some((session.host.clone(), project.project.root.clone()))
        });
        let (host, project_path) =
            found.with_context(|| format!("the Inbox has no live session {session_id}"))?;
        Ok(Self { inbox, host, session: session_id.to_owned(), project_path })
    }

    pub(crate) fn inbox(&self) -> &Shared {
        &self.inbox
    }

    /// Deposit the agent's message `host_message_id`: its review thread and message in the
    /// Inbox, the same ones again for the same words.
    pub(crate) fn deposit(&self, host_message_id: &str, text: &str) -> Result<Deposit> {
        let mut target = ReviewTarget::chat_message(&self.host, &self.session, host_message_id, text);
        target.project_path = Some(self.project_path.clone());
        self.inbox.deposit(&target).context("depositing the reply in the Inbox")
    }
}
