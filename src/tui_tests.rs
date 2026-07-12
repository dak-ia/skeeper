use super::*;
use std::path::PathBuf;
use time::macros::datetime;
use uuid::Uuid;

fn make_session(name: &str, attached: bool) -> SessionMeta {
    SessionMeta {
        id: Uuid::from_u128(0x1234_5678_1234_5678_1234_5678_1234_5678),
        name: name.to_string(),
        cwd: PathBuf::from("/tmp"),
        shell: PathBuf::from("/bin/sh"),
        created_at: datetime!(2000-01-02 03:04:05 UTC),
        last_attached_at: None,
        server_pid: 12345,
        server_started_at: datetime!(2000-01-02 03:04:05 UTC),
        attached_client_pid: if attached { Some(1) } else { None },
    }
}

#[test]
fn session_line_text_shows_detached_state() {
    let s = make_session("mysess", false);
    let line = session_line_text(&s, UtcOffset::UTC);
    assert!(line.contains("mysess"));
    assert!(line.contains("detached"));
    assert!(line.contains("2000-01-02 03:04:05"));
}

#[test]
fn session_line_text_shows_attached_state() {
    let s = make_session("mysess", true);
    let line = session_line_text(&s, UtcOffset::UTC);
    assert!(line.contains("attached"));
}

#[test]
fn empty_input_returns_none() {
    // pick_sessionはTerminalGuardを立てる前に空リストで抜けるので、
    // TTYが無いテスト環境でも安全に呼べる
    let result = pick_session(&[]).unwrap();
    assert_eq!(result, None);
}

#[test]
fn empty_input_multi_returns_none() {
    let result = pick_sessions_multi(&[]).unwrap();
    assert_eq!(result, None);
}
