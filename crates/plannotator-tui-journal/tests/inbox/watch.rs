//! The event stream: resuming, filtering, gathering, and when a watch stops.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use plannotator_tui_journal::{Client, Error, Event, Inbox, Kind, Token, Watch};

use crate::Result;
use crate::support::{FakeInbox, Reply, Request, SERVER_SESSION, health};

const EVENTS: &str = "/api/inbox/device/events";

fn hello(session: &str, cursor: u64) -> String {
    format!("retry: 20\nevent: hello\ndata: {{\"serverSession\":\"{session}\",\"cursor\":{cursor}}}\n\n")
}

fn record(seq: u64, kind: &str, thread: &str) -> String {
    format!(
        "id: {seq}\nevent: record\ndata: {{\"seq\":{seq},\"kind\":\"{kind}\",\"id\":\"r{seq}\",\"thread_id\":\"{thread}\"}}\n\n"
    )
}

/// A stand-in whose n-th connection to the event stream gets `streams(n)`.
fn streaming(
    name: &str,
    streams: impl Fn(usize, &Request) -> Reply + Send + Sync + 'static,
) -> Result<FakeInbox> {
    let connections = AtomicUsize::new(0);
    FakeInbox::start(name, move |request| match request.path.as_str() {
        EVENTS => streams(connections.fetch_add(1, Ordering::SeqCst), request),
        "/api/inbox/health" => health(SERVER_SESSION),
        _ => Reply::Json(404, serde_json::json!({ "code": "device_route_not_found" })),
    })
}

/// Events until `done` says enough, or two seconds pass.
fn collect(watch: &Watch, done: impl Fn(&[Event]) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut events = Vec::new();
    while !done(&events) && Instant::now() < deadline {
        events.extend(watch.next_timeout(Duration::from_millis(50)));
    }
    events
}

fn seqs(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .flat_map(|event| match event {
            Event::Records(records) => records.iter().map(|r| r.seq).collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn watch(inbox: &FakeInbox, thread: Option<&str>) -> Result<Watch> {
    Ok(Client::new(&inbox.data_dir, Token::new("tok_device")).watch(None, thread)?)
}

#[test]
fn a_cut_stream_resumes_from_the_last_seq_without_loss_or_double() -> Result {
    let inbox = streaming("resume", |n, _| match n {
        0 => Reply::Stream {
            text: hello("ss_one", 10) + &record(11, "message", "t") + &record(12, "message", "t"),
            hold: false,
        },
        // The Inbox replays from Last-Event-ID; one line overlaps.
        _ => Reply::Stream {
            text: hello("ss_one", 13) + &record(12, "message", "t") + &record(13, "annotation", "t"),
            hold: true,
        },
    })?;
    let watch = watch(&inbox, None)?;
    let events = collect(&watch, |events| seqs(events).len() >= 3);
    assert_eq!(seqs(&events), vec![11, 12, 13]);
    let connections = inbox.requests_to(EVENTS);
    let resumed: Vec<Option<&str>> = connections.iter().map(|r| r.header("last-event-id")).collect();
    assert_eq!(resumed.get(..2), Some([None, Some("12")].as_slice()));
    Ok(())
}

#[test]
fn only_the_watched_threads_records_are_told_and_unknown_kinds_are_skipped() -> Result {
    let fixture = include_str!("../fixtures/v1/events.sse");
    let inbox = streaming("filter", move |_, _| Reply::Stream { text: fixture.to_owned(), hold: true })?;
    let watch = watch(&inbox, Some("msg_a1"))?;
    let events = collect(&watch, |events| seqs(events).len() >= 3);
    let told: Vec<(u64, Kind)> = events
        .iter()
        .flat_map(|event| match event {
            Event::Records(records) => records.iter().map(|r| (r.seq, r.kind)).collect(),
            _ => Vec::new(),
        })
        .collect();
    // A message's thread read from the record it carries; another thread's comment, a kind
    // this client does not know, a project and a question with no thread left out.
    assert_eq!(told, vec![(41, Kind::Message), (43, Kind::CommentReply), (46, Kind::CommentSend)]);
    Ok(())
}

#[test]
fn records_that_arrive_together_are_told_once() -> Result {
    let burst = hello("ss_one", 0)
        + &record(1, "annotation", "t")
        + &record(2, "comment_send", "t")
        + &record(3, "message", "t");
    let inbox = streaming("gather", move |_, _| Reply::Stream { text: burst.clone(), hold: true })?;
    let watch = watch(&inbox, None)?;
    let events = collect(&watch, |events| seqs(events).len() >= 3);
    let batches: Vec<usize> =
        events.iter().filter_map(|e| if let Event::Records(r) = e { Some(r.len()) } else { None }).collect();
    assert_eq!(batches, vec![3]);
    Ok(())
}

#[test]
fn a_restarted_inbox_is_told() -> Result {
    let inbox = streaming("restart", |n, _| match n {
        0 => Reply::Stream { text: hello("ss_one", 5), hold: false },
        _ => Reply::Stream { text: hello("ss_two", 7), hold: true },
    })?;
    let watch = watch(&inbox, None)?;
    let events = collect(&watch, |events| events.iter().any(|e| matches!(e, Event::Restarted { .. })));
    assert!(events.contains(&Event::Restarted { server_session: "ss_two".into(), cursor: 7 }));
    Ok(())
}

#[test]
fn a_newer_contract_stops_the_watch() -> Result {
    let line = "id: 2\nevent: record\ndata: {\"v\":2,\"seq\":2,\"kind\":\"message\",\"id\":\"m\",\"thread_id\":\"t\"}\n\n";
    let inbox =
        streaming("newer", move |_, _| Reply::Stream { text: hello("ss_one", 1) + line, hold: true })?;
    let watch = watch(&inbox, None)?;
    let events = collect(&watch, |events| events.iter().any(|e| matches!(e, Event::Stopped(_))));
    assert!(events.iter().any(|e| matches!(e, Event::Stopped(Error::Newer { v: 2, .. }))));
    Ok(())
}

#[test]
fn a_refused_token_stops_the_watch_without_retrying() -> Result {
    let inbox = streaming("refused", |_, _| {
        Reply::Json(401, serde_json::json!({ "error": "removed", "code": "device_revoked" }))
    })?;
    let watch = watch(&inbox, None)?;
    let events = collect(&watch, |events| events.iter().any(|e| matches!(e, Event::Stopped(_))));
    assert!(events.iter().any(|e| matches!(e, Event::Stopped(Error::Unpaired { .. }))));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(inbox.requests_to(EVENTS).len(), 1);
    Ok(())
}

#[test]
fn a_line_the_client_cannot_read_is_passed_over() -> Result {
    // An Inbox whose stream has no case for a kind writes `data: undefined`; a later kind may
    // carry fields of other shapes. Neither stops the watch.
    let odd = "id: 2\nevent: record\ndata: undefined\n\n\
               id: 3\nevent: record\ndata: {\"seq\":3,\"kind\":\"later\",\"id\":7,\"message\":\"text\"}\n\n";
    let text = hello("ss_one", 1) + odd + &record(4, "message", "t");
    let inbox = streaming("odd", move |_, _| Reply::Stream { text: text.clone(), hold: true })?;
    let watch = watch(&inbox, None)?;
    let events = collect(&watch, |events| !seqs(events).is_empty());
    assert_eq!(seqs(&events), vec![4]);
    assert!(!events.iter().any(|e| matches!(e, Event::Stopped(_))));
    Ok(())
}

#[test]
fn an_inbox_that_numbers_from_lower_is_followed_from_its_cursor() -> Result {
    // A new data folder: the restarted Inbox's seq starts below what was seen.
    let inbox = streaming("renumbered", |n, _| match n {
        0 => Reply::Stream { text: hello("ss_one", 10) + &record(11, "message", "t"), hold: false },
        _ => Reply::Stream { text: hello("ss_two", 2) + &record(3, "message", "t"), hold: true },
    })?;
    let watch = watch(&inbox, None)?;
    let events = collect(&watch, |events| seqs(events).len() >= 2);
    assert_eq!(seqs(&events), vec![11, 3]);
    Ok(())
}
