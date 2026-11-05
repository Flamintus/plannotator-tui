//! `last` through the Inbox: the reply under review is deposited in its session's review
//! thread (`crate::inbox::import`) when it is opened for review, never while the picker
//! only previews it. A reply the Inbox refuses stays as it was, its feedback sent to the
//! agent's pane: never both.

use anyhow::Result;
use plannotator_tui_schema::Provenance;

use super::super::{App, Mode};
use super::InboxReview;
use crate::inbox::Target;
use crate::inbox::import::Depositor;

impl App {
    /// Review `last`'s replies through the Inbox when `depositor` has one; say why not when
    /// it failed; do nothing with the Inbox off.
    pub(crate) fn review_in_inbox(&mut self, depositor: Result<Option<Depositor>>) {
        match depositor {
            Ok(None) => {}
            Ok(Some(depositor)) => {
                self.inbox = Some(InboxReview { documents: Vec::new(), depositor: Some(depositor) });
                if self.mode != Mode::Pick {
                    self.deposit_open();
                }
            }
            Err(err) => self.inbox_fallback(&err),
        }
    }

    /// Deposit the open reply, unless it is in the Inbox already or has notes of its own,
    /// which only the agent's pane can still take.
    pub(in crate::app) fn deposit_open(&mut self) {
        let Some(depositor) = self.inbox.as_ref().and_then(|review| review.depositor.as_ref()) else {
            return;
        };
        if self.open.store.len() > 0 {
            return;
        }
        let Provenance::AgentMessage { message_id: Some(message_id), .. } = &self.open.source.provenance
        else {
            return;
        };
        let text = self.open.doc.source.clone();
        let deposited = depositor.deposit(message_id, &text).and_then(|deposit| {
            let target = Target::Message { thread_id: deposit.thread_id, message_id: deposit.message_id };
            let name = format!("{} \u{b7} Inbox", self.open.source.name);
            Ok((self.open_document(&target, name, text)?, deposit.created))
        });
        match deposited {
            Ok((open, created)) => {
                self.open = open;
                self.derive_send_state();
                let state = if created { "is now" } else { "was already" };
                self.status = Some(format!("this reply {state} in the Inbox"));
            }
            Err(err) => self.inbox_fallback(&err),
        }
    }

    /// Say why the Inbox is out of this review, after whatever the footer already says.
    fn inbox_fallback(&mut self, err: &anyhow::Error) {
        let note = format!("Inbox: {err:#} \u{b7} feedback goes to the agent as before");
        self.status = Some(match self.status.take() {
            Some(status) => format!("{status} \u{b7} {note}"),
            None => note,
        });
    }
}
