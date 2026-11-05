//! The wire types against the contract fixtures.

use plannotator_tui_inbox::{ReplyNode, ThreadNode, ThreadView};
use serde_json::Value;

use crate::Result;

/// Every field a node carries is one the types name: nothing falls into `other`.
fn fully_typed(nodes: &[ThreadNode]) -> bool {
    let reply = |reply: &ReplyNode| reply.other.is_empty() && fully_typed(&reply.sub_threads);
    nodes
        .iter()
        .all(|node| node.other.is_empty() && node.replies.iter().all(reply) && fully_typed(&node.sub_threads))
}

#[test]
fn the_thread_view_reads_exactly_what_the_inbox_computes() -> Result {
    let fixture: Value = serde_json::from_str(include_str!("../fixtures/v1/thread-view.json"))?;
    let view: ThreadView = serde_json::from_value(fixture.clone())?;
    assert_eq!(serde_json::to_value(&view)?, fixture, "no field lost, none invented");
    assert!(view.other.is_empty() && fully_typed(&view.threads), "the types mirror the view");
    Ok(())
}
