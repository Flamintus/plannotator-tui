#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::{Cli, Journal as _, Stamp, journal_path};
use crate::error::Error;
use crate::requests::{IdempotencyKey, NewComment, Placement, ReviewTarget, SendMode};
use crate::view::{Anchor, CommentKind, State, Status};

const VIEW: &str = include_str!("../../tests/fixtures/v1/view.json");
const SEND: &str = include_str!("../../tests/fixtures/v1/send.json");

fn target() -> ReviewTarget {
    ReviewTarget::Last { host: "claude-code".into(), session: "ses_a".into() }
}

/// A fresh folder under the temp dir.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-journal-{name}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn the_view_plannotator_computed_reads_whole() {
    let viewed: crate::Viewed = serde_json::from_str(VIEW).expect("the fixture reads");
    assert_eq!(viewed.review.len(), "last-".len() + 24);
    let view = viewed.view;
    assert_eq!((view.v, view.round), (1, 2));
    let ids: Vec<&str> = view.threads.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(ids, ["c1", "c2", "d1"]);
    let c1 = &view.threads[0];
    assert_eq!(c1.place.on, Anchor::Message { message_id: "m1".into() });
    assert_eq!(c1.status, Some(Status::Replied));
    let agent = &c1.replies[0];
    assert_eq!(agent.author_name.as_deref(), Some("Claude Code"));
    let sub = &agent.sub_threads[0];
    assert_eq!((sub.id.as_str(), sub.draft), ("s1", true));
    assert_eq!(sub.place.parent.as_ref().map(|parent| parent.quote_offset), Some(27));
    assert!(c1.replies[1].draft, "the person's reply waits for the next send");
    assert_eq!(c1.kind, CommentKind::Comment);
    let c2 = &view.threads[1];
    assert_eq!((c2.state, c2.folded, c2.kind), (State::Resolved, true, CommentKind::Deletion));
    assert!(view.threads[2].draft);
}

#[test]
fn a_send_reads_with_the_text_for_the_agent() {
    let sent: crate::Sent = serde_json::from_str(SEND).expect("the fixture reads");
    assert_eq!((sent.send.round, sent.send.mode), (1, SendMode::Round));
    assert_eq!(sent.send.comments, ["c1", "c2"]);
    assert_eq!(sent.send.to.session, "ses_a");
    assert!(sent.text.starts_with("# Comments on your work"));
    assert!(sent.text.contains(&format!("plannotator reply --review {}", sent.review)));
}

#[test]
fn a_stamp_is_read_only_for_a_reviews_key() {
    let data = scratch("stamp");
    let mut cli = Cli::new("plannotator");
    assert_eq!(cli.stamp("last-0123456789abcdef01234567"), None, "no data directory known before the probe");
    cli.data_dir = Some(data.clone());
    let key = "last-0123456789abcdef01234567";
    assert_eq!(cli.stamp(key), None, "no journal yet");
    let path = journal_path(&data, key).expect("a key");
    std::fs::create_dir_all(path.parent().expect("folder")).expect("folder");
    std::fs::write(&path, "{}\n").expect("journal");
    let first = cli.stamp(key).expect("a journal");
    std::fs::write(&path, "{}\n{}\n").expect("written again");
    assert_ne!(cli.stamp(key), Some(first));
    assert_eq!(journal_path(&data, "../last-0123456789abcdef01234567"), None);
    assert_eq!(journal_path(&data, "last-0123456789ABCDEF01234567"), None);
    assert_eq!(Stamp::new(3, None), Stamp::new(3, None));
    std::fs::remove_dir_all(data).expect("cleanup");
}

#[test]
fn plannotator_is_found_on_the_path_only_where_it_runs() {
    let dir = scratch("find");
    let path = std::env::join_paths([dir.join("nowhere"), dir.clone()]).expect("a PATH");
    assert_eq!(Cli::find(&path), None);
    let name = if cfg!(windows) { "plannotator.exe" } else { "plannotator" };
    std::fs::write(dir.join(name), "").expect("file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(Cli::find(&path), None, "a file that does not run is no program");
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    assert_eq!(Cli::find(&path), Some(dir.join(name)));
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn a_program_that_is_not_there_is_said() {
    let err = Cli::new("/nonexistent/plannotator").view(&target()).expect_err("no program");
    assert!(matches!(err, Error::Spawn { .. }), "{err:?}");
}

/// A stand-in `plannotator`: it keeps its arguments and stdin beside it, then runs `body`.
#[cfg(unix)]
fn stand_in(name: &str, body: &str) -> (Cli, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch(name);
    let program = dir.join("plannotator");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{dir}/args\"\ncat > \"{dir}/stdin\"\n{body}\n",
        dir = dir.display()
    );
    std::fs::write(&program, script).expect("script");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (Cli::new(&program), dir)
}

#[cfg(unix)]
fn read(dir: &std::path::Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).expect(file)
}

#[cfg(unix)]
#[test]
fn a_call_names_its_action_and_gives_its_json_on_stdin() {
    let fixture =
        std::env::temp_dir().join(format!("plannotator-tui-journal-view-{}.json", std::process::id()));
    std::fs::write(&fixture, VIEW).expect("fixture");
    let (cli, dir) = stand_in("call", &format!("cat \"{}\"", fixture.display()));
    let viewed = cli.view(&target()).expect("the view");
    assert_eq!(viewed.view.threads.len(), 3);
    assert_eq!(read(&dir, "args"), "threads\nview\n-\n");
    let sent: Value = serde_json::from_str(&read(&dir, "stdin")).expect("JSON on stdin");
    assert_eq!(sent, json!({ "target": { "kind": "last", "host": "claude-code", "session": "ses_a" } }));
    let comment = NewComment {
        id: "local_1".into(),
        text: "Why?".into(),
        placement: Placement::Under { re: "c1".into() },
    };
    cli.comment(&target(), &comment).expect("written");
    assert_eq!(read(&dir, "args"), "threads\ncomment\n-\n");
    let sent: Value = serde_json::from_str(&read(&dir, "stdin")).expect("JSON on stdin");
    assert_eq!(sent["re"], "c1");
    assert_eq!(sent["target"]["session"], "ses_a");
    cli.resolve(&target(), "c1", false).expect("reopened");
    assert_eq!(read(&dir, "args"), "threads\nreopen\n-\n");
    cli.close(&target()).expect("closed");
    assert_eq!(read(&dir, "args"), "threads\nclose\n-\n");
    std::fs::remove_dir_all(dir).expect("cleanup");
    std::fs::remove_file(fixture).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn a_send_carries_the_press_key_and_a_now_send_its_drafts() {
    let (cli, dir) = stand_in("send", &format!("printf '%s' '{}'", SEND.trim().replace('\'', "'\\''")));
    let key = IdempotencyKey::new();
    let sent = cli.send(&target(), SendMode::Round, &[], None, &key).expect("sent");
    assert!(!sent.replayed);
    let asked: Value = serde_json::from_str(&read(&dir, "stdin")).expect("JSON on stdin");
    assert_eq!(asked["mode"], "round");
    assert_eq!(asked["idempotency_key"], key.as_str());
    assert!(asked.get("ids").is_none(), "the end of a round carries every draft left");
    assert!(asked.get("to").is_none(), "a session's review names its session itself");
    cli.send(&target(), SendMode::Now, &["d1".to_owned()], None, &key).expect("sent");
    let asked: Value = serde_json::from_str(&read(&dir, "stdin")).expect("JSON on stdin");
    assert_eq!(asked["ids"], json!(["d1"]));
    let err = cli.send(&target(), SendMode::Now, &[], None, &key).expect_err("names no draft");
    assert!(matches!(err, Error::Invalid(_)));
    let file = ReviewTarget::File { path: "/repo/docs".into() };
    let to = crate::SendTo { host: Some("claude-code".into()), session: "ses_b".into() };
    cli.send(&file, SendMode::Round, &[], Some(&to), &key).expect("sent");
    let asked: Value = serde_json::from_str(&read(&dir, "stdin")).expect("JSON on stdin");
    assert_eq!(asked["target"], json!({ "kind": "file", "path": "/repo/docs" }));
    assert_eq!(asked["to"], json!({ "host": "claude-code", "session": "ses_b" }));
    let again = cli.resend(&file, "snd_1").expect("its text again");
    assert_eq!(again.send.comments, sent.send.comments);
    assert_eq!(read(&dir, "args"), "threads\nsend\n-\n");
    let asked: Value = serde_json::from_str(&read(&dir, "stdin")).expect("JSON on stdin");
    assert_eq!(asked, json!({ "target": { "kind": "file", "path": "/repo/docs" }, "id": "snd_1" }));
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn refusals_and_bad_calls_keep_the_clis_words() {
    let (cli, dir) = stand_in("refused", "echo 'not_found: No comment c9.' >&2\nexit 1");
    let err = cli.remove(&target(), "c9").expect_err("refused");
    assert_eq!(err.code(), Some("not_found"));
    assert_eq!(err.to_string(), "threads remove: No comment c9. (not_found)");
    std::fs::remove_dir_all(dir).expect("cleanup");
    let (cli, dir) = stand_in("usage", "echo 'Unknown argument: view' >&2\nexit 2");
    let err = cli.view(&target()).expect_err("not read");
    assert_eq!(err, Error::Usage { action: "view".into(), detail: "Unknown argument: view".into() });
    std::fs::remove_dir_all(dir).expect("cleanup");
    let (cli, dir) = stand_in("newer", r#"echo '{"review":"last-0","view":{"v":2,"round":1,"threads":[]}}'"#);
    assert_eq!(cli.view(&target()), Err(Error::Newer { what: "the thread view".into(), v: 2 }));
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn a_call_that_does_not_answer_is_stopped() {
    let (cli, dir) = stand_in("slow", "sleep 5");
    let cli = cli.with_timeout(Duration::from_millis(200));
    let started = std::time::Instant::now();
    assert_eq!(cli.view(&target()), Err(Error::Timeout { action: "view".into() }));
    assert!(started.elapsed() < Duration::from_secs(3), "stopped, not waited for");
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn only_a_plannotator_with_the_journals_actions_passes_the_probe() {
    let script = "case \"$1\" in\n  data-dir) echo /data/plannotator ;;\n  *) echo '  plannotator threads <action> [<json> | -]' ;;\nesac";
    let (cli, dir) = stand_in("probe", script);
    let cli = cli.probe().expect("has them");
    assert_eq!(read(&dir, "args"), "data-dir\n", "then asks where its journals are");
    assert_eq!(cli.data_dir.as_deref(), Some(std::path::Path::new("/data/plannotator")));
    std::fs::remove_dir_all(dir).expect("cleanup");
    let (cli, dir) = stand_in("old", "echo 'Usage: plannotator [file]'");
    assert!(matches!(cli.probe(), Err(Error::Usage { .. })));
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn the_live_mode_reads_off_from_an_older_view_and_as_the_view_says_it_otherwise() {
    let viewed: crate::Viewed = serde_json::from_str(VIEW).expect("the fixture reads");
    assert!(!viewed.view.live_mode, "a view from before the Live mode says nothing: off");
    let mut value: Value = serde_json::from_str(VIEW).expect("JSON");
    value["view"]["live_mode"] = json!(true);
    let viewed: crate::Viewed = serde_json::from_value(value).expect("reads");
    assert!(viewed.view.live_mode);
    assert!(!viewed.view.other.contains_key("live_mode"), "read as the field, not kept aside");
}

#[cfg(unix)]
#[test]
fn the_live_mode_is_turned_on_and_off_by_its_own_verb() {
    let (cli, dir) = stand_in("live-mode", "echo '{}'");
    cli.live_mode(&target(), true).expect("on");
    assert_eq!(read(&dir, "args"), "threads\nlive-mode\non\n-\n");
    let sent: Value = serde_json::from_str(&read(&dir, "stdin")).expect("JSON on stdin");
    assert_eq!(sent, json!({ "target": { "kind": "last", "host": "claude-code", "session": "ses_a" } }));
    cli.live_mode(&target(), false).expect("off");
    assert_eq!(read(&dir, "args"), "threads\nlive-mode\noff\n-\n");
    std::fs::remove_dir_all(dir).expect("cleanup");
}
