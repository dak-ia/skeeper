use super::*;

use std::path::PathBuf;
use time::UtcOffset;
use time::macros::datetime;
use uuid::Uuid;

use crate::session::{ClientInfo, SCHEMA_VERSION_CURRENT, SessionMeta};

fn session_two_clients_outdated() -> SessionMeta {
    SessionMeta {
        id: Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000),
        name: "brave-otter".to_string(),
        cwd: PathBuf::from("/home/u/proj"),
        shell: PathBuf::from("/bin/bash"),
        created_at: datetime!(2000-01-02 03:04:05 UTC),
        last_attached_at: Some(datetime!(2000-01-02 04:00:00 UTC)),
        server_pid: 999,
        server_started_at: datetime!(2000-01-02 03:00:00 UTC),
        schema_version: SCHEMA_VERSION_CURRENT,
        ipc_protocol_version: 0,
        attached_clients: vec![
            ClientInfo {
                pid: 1001,
                tty: Some("/dev/pts/1".to_string()),
                ssh_connection: None,
                attached_at: datetime!(2000-01-02 03:04:05 UTC),
            },
            ClientInfo {
                pid: 1002,
                tty: Some("/dev/pts/2".to_string()),
                ssh_connection: Some("10.0.0.1 22 10.0.0.2 22".to_string()),
                attached_at: datetime!(2000-01-02 03:30:00 UTC),
            },
        ],
    }
}

fn session_zero_clients_current() -> SessionMeta {
    SessionMeta {
        id: Uuid::from_u128(0xd3fa_7100_0000_0000_0000_0000_0000_0000),
        name: "lucky-fox".to_string(),
        cwd: PathBuf::from("/tmp"),
        shell: PathBuf::from("/bin/bash"),
        created_at: datetime!(2000-01-02 03:04:05 UTC),
        last_attached_at: None,
        server_pid: 999,
        server_started_at: datetime!(2000-01-02 03:00:00 UTC),
        schema_version: SCHEMA_VERSION_CURRENT,
        ipc_protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
        attached_clients: Vec::new(),
    }
}

fn session_one_client_none_fields() -> SessionMeta {
    SessionMeta {
        id: Uuid::from_u128(0xaabb_ccdd_0000_0000_0000_0000_0000_0000),
        name: "minimal-cat".to_string(),
        cwd: PathBuf::from("/tmp"),
        shell: PathBuf::from("/bin/bash"),
        created_at: datetime!(2000-01-02 03:04:05 UTC),
        last_attached_at: Some(datetime!(2000-01-02 03:04:05 UTC)),
        server_pid: 999,
        server_started_at: datetime!(2000-01-02 03:00:00 UTC),
        schema_version: SCHEMA_VERSION_CURRENT,
        ipc_protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
        attached_clients: vec![ClientInfo {
            pid: 555,
            tty: None,
            ssh_connection: None,
            attached_at: datetime!(2000-01-02 03:04:05 UTC),
        }],
    }
}

#[test]
fn run_returns_ok_when_runtime_dir_is_empty() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    // HOMEフォールバックにも同じtempを向ける: XDG側が無視される変更が入ってもテスト隔離を保つ
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }
    run(ListArgs { detail: false }).unwrap();
}

#[test]
fn session_line_shows_outdated_when_ipc_version_below_current() {
    let s = session_two_clients_outdated();
    let line = render_session_line(&s, UtcOffset::UTC);
    assert_eq!(
        line,
        "550e8400  brave-otter           attached (2)   2000-01-02 03:04:05  2000-01-02 04:00:00  outdated  /home/u/proj"
    );
}

#[test]
fn session_line_shows_dash_proto_and_detached_state_for_zero_client_current_server() {
    let s = session_zero_clients_current();
    let line = render_session_line(&s, UtcOffset::UTC);
    assert_eq!(
        line,
        "d3fa7100  lucky-fox             detached       2000-01-02 03:04:05  -                    -         /tmp"
    );
}

#[test]
fn client_sub_table_is_empty_when_no_attached_clients() {
    let s = session_zero_clients_current();
    let lines = render_client_sub_table(&s, None, UtcOffset::UTC);
    assert!(lines.is_empty());
}

#[test]
fn client_sub_table_shows_pid_tty_ssh_and_attach_time_with_me_marker() {
    let s = session_two_clients_outdated();
    let lines = render_client_sub_table(&s, Some(1002), UtcOffset::UTC);
    assert_eq!(
        lines,
        vec![
            "          PID      TTY           SSH_CONNECTION            ATTACHED".to_string(),
            "          1001     /dev/pts/1    -                         2000-01-02 03:04:05"
                .to_string(),
            "          1002     /dev/pts/2    10.0.0.1 22 10.0.0.2 22   2000-01-02 03:30:00 (me)"
                .to_string(),
        ]
    );
}

#[test]
fn client_sub_table_shows_dash_for_none_tty_and_none_ssh() {
    let s = session_one_client_none_fields();
    let lines = render_client_sub_table(&s, None, UtcOffset::UTC);
    assert_eq!(
        lines,
        vec![
            "          PID      TTY           SSH_CONNECTION            ATTACHED".to_string(),
            "          555      -             -                         2000-01-02 03:04:05"
                .to_string(),
        ]
    );
}
