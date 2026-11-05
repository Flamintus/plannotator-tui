//! Discovery, local pairing and the kept device token, against the stand-in Inbox.

use plannotator_tui_journal::{Device, Error, Inbox, Paired, Token, connect, discover, token_path};
use serde_json::{Map, json};

use crate::Result;
use crate::support::{
    CONNECTION_TOKEN, DEVICE_TOKEN, FakeInbox, PAIR_LOCAL, Reply, SERVER_SESSION, client_of, door, field,
    health, scratch, write_registry,
};

#[test]
fn discovery_reads_the_registry_and_checks_the_inboxs_health() -> Result {
    let inbox = FakeInbox::start("discover", door)?;
    let found = discover(&inbox.data_dir)?;
    assert_eq!(found.base_url, format!("http://127.0.0.1:{}", inbox.port));
    assert_eq!((found.server_session.as_str(), found.version.as_str()), (SERVER_SESSION, "0.28.49-test"));
    assert!(!format!("{found:?}").contains(CONNECTION_TOKEN), "the connection token is never printed");
    Ok(())
}

#[test]
fn an_inbox_is_running_only_when_its_port_answers_the_registrys_session() -> Result {
    let never = scratch("never")?;
    assert!(matches!(discover(&never), Err(Error::NotRunning(_))), "no registry");
    let other = FakeInbox::start("other", |_| health("ss_other"))?;
    assert!(matches!(discover(&other.data_dir), Err(Error::NotRunning(_))), "another Inbox on the port");
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = closed.local_addr()?.port();
    drop(closed);
    write_registry(&never, port, SERVER_SESSION)?;
    assert!(matches!(discover(&never), Err(Error::NotRunning(_))), "nothing listens");
    Ok(())
}

#[test]
fn the_device_token_goes_only_to_a_running_inbox() -> Result {
    // The registry stays when the Inbox stops; whatever holds its port then gets no token.
    let squatter = FakeInbox::start("squatter", |request| match request.path.as_str() {
        "/api/inbox/health" => health("ss_other"),
        _ => door(request),
    })?;
    assert!(matches!(client_of(&squatter).threads(), Err(Error::NotRunning(_))));
    assert!(squatter.requests().iter().all(|r| r.header("authorization").is_none()));
    Ok(())
}

#[test]
fn local_pairing_presents_the_connection_token_as_a_terminal() -> Result {
    let inbox = FakeInbox::start("pair", door)?;
    connect(&inbox.data_dir, "tui")?;
    let pairing = inbox.requests_to(PAIR_LOCAL);
    let [pairing] = pairing.as_slice() else { return Err("one pairing".into()) };
    assert_eq!(pairing.header("authorization"), Some(format!("Bearer {CONNECTION_TOKEN}").as_str()));
    let body = pairing.json();
    assert_eq!((field(&body, "platform"), field(&body, "name")), (json!("terminal"), json!("tui")));
    Ok(())
}

#[test]
fn the_device_token_is_kept_for_the_next_connection() -> Result {
    let inbox = FakeInbox::start("kept", door)?;
    connect(&inbox.data_dir, "tui")?;
    connect(&inbox.data_dir, "tui")?.health()?;
    assert_eq!(inbox.requests_to(PAIR_LOCAL).len(), 1, "no second pairing");
    let kept = Paired::load(&token_path(&inbox.data_dir))?.ok_or("token kept")?;
    assert_eq!(kept.token, Token::new(DEVICE_TOKEN));
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_kept_token_is_the_owners_only() -> Result {
    use std::os::unix::fs::PermissionsExt as _;
    let inbox = FakeInbox::start("private", door)?;
    connect(&inbox.data_dir, "tui")?;
    let path = token_path(&inbox.data_dir);
    assert_eq!(std::fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(path.parent().ok_or("folder")?)?.permissions().mode() & 0o777, 0o700);
    Ok(())
}

#[test]
fn a_revoked_token_is_forgotten_and_replaced() -> Result {
    let inbox = FakeInbox::start("revoked", |request| match request.path.as_str() {
        "/api/inbox/device/health" if request.header("authorization") == Some("Bearer tok_old") => {
            Reply::Json(401, json!({ "error": "removed", "code": "device_revoked" }))
        }
        _ => door(request),
    })?;
    let old =
        Device { id: "dev_0".into(), name: "tui".into(), platform: "terminal".into(), other: Map::new() };
    Paired { token: Token::new("tok_old"), device: old }.save(&token_path(&inbox.data_dir))?;
    connect(&inbox.data_dir, "tui")?;
    assert_eq!(Paired::load(&token_path(&inbox.data_dir))?.map(|p| p.token), Some(Token::new(DEVICE_TOKEN)));
    Ok(())
}

#[test]
fn an_unreadable_token_file_is_replaced() -> Result {
    let inbox = FakeInbox::start("unreadable", door)?;
    let path = token_path(&inbox.data_dir);
    std::fs::create_dir_all(path.parent().ok_or("folder")?)?;
    std::fs::write(&path, "{ cut sho")?;
    connect(&inbox.data_dir, "tui")?;
    assert_eq!(Paired::load(&path)?.map(|p| p.token), Some(Token::new(DEVICE_TOKEN)));
    Ok(())
}

#[test]
fn a_refused_local_pairing_is_no_reason_to_pair_again() -> Result {
    // Today's door with Phones on reads the connection token as an unknown device token.
    let inbox = FakeInbox::start("pair-refused", |request| match request.path.as_str() {
        PAIR_LOCAL => Reply::Json(401, json!({ "error": "unknown", "code": "device_token_invalid" })),
        _ => door(request),
    })?;
    assert!(matches!(connect(&inbox.data_dir, "tui"), Err(Error::Refused { status: 401, .. })));
    Ok(())
}
