//! What a terminal writes. The client mints no comment id: the caller's own id becomes the
//! journal's (`^[A-Za-z0-9_.:-]{1,200}$`), so the note and its comment share one id.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::view::Anchor;

/// The key a send is answered under: the journal keeps it on the send, and a send made again
/// under it writes nothing and gives the first one back, its text included. One per press:
/// a press tried again after an error, under the same key, cannot send twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// A fresh key: unique per process and call.
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        Self(format!("tui_{nanos:x}_{:x}_{:x}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for IdempotencyKey {
    fn default() -> Self {
        Self::new()
    }
}

/// What a review is on, which names its journal (`ReviewTarget` in Plannotator's
/// `review-journal.ts`). Every client of the same target finds the same review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewTarget {
    /// An agent session's last messages: `host` as Plannotator names it (`claude-code`), and
    /// the host's own session id.
    Last { host: String, session: String },
}

/// Where the person's comment goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// On the target's words: `quote`, as selected, at `on`. `deletion`: the person asks for
    /// the words to go.
    On { on: Anchor, quote: String, deletion: bool },
    /// The person's reply under sent comment `re` (a sent reply's id answers its comment).
    Under { re: String },
    /// A sub-thread on `quote`, a passage of sent message `parent` (a comment, a reply or a
    /// sub-thread), starting `quote_offset` into its words as a passage counts them
    /// (whitespace runs as one); the journal finds the first occurrence when it is `None`.
    Passage { parent: String, quote: String, quote_offset: Option<u64> },
}

/// The person's comment, reply or sub-thread: a draft until a send carries it. Writing the
/// same id again edits the draft; a sent one is not edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewComment {
    pub id: String,
    pub text: String,
    pub placement: Placement,
}

impl NewComment {
    /// The fields `threads comment` reads, beside the review's.
    pub(crate) fn fields(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        fields.insert("id".to_owned(), json!(self.id));
        fields.insert("text".to_owned(), json!(self.text));
        match &self.placement {
            Placement::On { on, quote, deletion } => {
                fields.insert("on".to_owned(), json!(on));
                fields.insert("quote".to_owned(), json!(quote));
                if *deletion {
                    fields.insert("type".to_owned(), json!("deletion"));
                }
            }
            Placement::Under { re } => {
                fields.insert("re".to_owned(), json!(re));
            }
            Placement::Passage { parent, quote, quote_offset } => {
                fields.insert("parent".to_owned(), json!(parent));
                fields.insert("quote".to_owned(), json!(quote));
                if let Some(offset) = quote_offset {
                    fields.insert("quote_offset".to_owned(), json!(offset));
                }
            }
        }
        fields
    }
}

/// `now` sends at once and leaves the round open; `round` is the person's end of a round,
/// carrying every draft left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendMode {
    Now,
    Round,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_placement_writes_the_fields_the_cli_reads() {
        let on = NewComment {
            id: "local_1".into(),
            text: String::new(),
            placement: Placement::On {
                on: Anchor::Message { message_id: "m1".into() },
                quote: "the panel".into(),
                deletion: true,
            },
        };
        assert_eq!(
            Value::Object(on.fields()),
            json!({ "id": "local_1", "text": "", "on": { "kind": "message", "message_id": "m1" },
                    "quote": "the panel", "type": "deletion" })
        );
        let under = NewComment {
            id: "r".into(),
            text: "Yes.".into(),
            placement: Placement::Under { re: "c".into() },
        };
        assert_eq!(Value::Object(under.fields()), json!({ "id": "r", "text": "Yes.", "re": "c" }));
        let passage = NewComment {
            id: "s".into(),
            text: "Why?".into(),
            placement: Placement::Passage {
                parent: "rep_1".into(),
                quote: "a word".into(),
                quote_offset: Some(4),
            },
        };
        assert_eq!(
            Value::Object(passage.fields()),
            json!({ "id": "s", "text": "Why?", "parent": "rep_1", "quote": "a word", "quote_offset": 4 })
        );
    }

    #[test]
    fn a_last_target_is_named_by_its_host_and_session() {
        let target = ReviewTarget::Last { host: "claude-code".into(), session: "ses_a".into() };
        assert_eq!(json!(target), json!({ "kind": "last", "host": "claude-code", "session": "ses_a" }));
    }
}
