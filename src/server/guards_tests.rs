use super::{AttachStateGuard, SessionFileGuard};
use crate::session::{self, SessionMeta};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use time::macros::datetime;
use uuid::Uuid;

fn fixture_meta(attached_pid: Option<u32>) -> SessionMeta {
    let ts = datetime!(2000-01-02 03:04:05 UTC);
    SessionMeta {
        id: Uuid::from_u128(0x1),
        name: "sess".to_string(),
        cwd: PathBuf::from("/"),
        shell: PathBuf::from("/bin/sh"),
        created_at: ts,
        last_attached_at: None,
        server_pid: 1,
        server_started_at: ts,
        attached_client_pid: attached_pid,
    }
}

#[test]
fn session_file_guard_drop_removes_meta_socket_and_ctl() {
    let dir = tempdir().unwrap();
    let meta = dir.path().join("m.json");
    let sock = dir.path().join("s.sock");
    let ctl = dir.path().join("c.ctl");
    std::fs::write(&meta, b"meta").unwrap();
    std::fs::write(&sock, b"sock").unwrap();
    std::fs::write(&ctl, b"ctl").unwrap();

    drop(SessionFileGuard {
        meta_path: &meta,
        socket_path: &sock,
        ctl_socket_path: &ctl,
    });

    assert!(!meta.exists());
    assert!(!sock.exists());
    assert!(!ctl.exists());
}

#[test]
fn session_file_guard_drop_ignores_missing_files_without_panic() {
    // 二重起動/クラッシュ後の起動で対象ファイルが最初から無いことがあるので、
    // すべて存在しなくてもdropはpanicしないことを保証する
    let dir = tempdir().unwrap();
    let meta = dir.path().join("absent_m.json");
    let sock = dir.path().join("absent_s.sock");
    let ctl = dir.path().join("absent_c.ctl");

    drop(SessionFileGuard {
        meta_path: &meta,
        socket_path: &sock,
        ctl_socket_path: &ctl,
    });
}

#[test]
fn attach_state_guard_disarm_keeps_active_client_and_meta_pid() {
    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("m.json");
    let meta_initial = fixture_meta(Some(4321));
    session::write_meta_atomic(&meta_path, &meta_initial).unwrap();

    let (stream_a, _stream_b) = UnixStream::pair().unwrap();
    let client = Arc::new(Mutex::new(stream_a));
    let active_client: Mutex<Option<Arc<Mutex<UnixStream>>>> =
        Mutex::new(Some(Arc::clone(&client)));
    let meta_state = Mutex::new(meta_initial);

    {
        let mut guard = AttachStateGuard {
            active_client: &active_client,
            meta: &meta_state,
            meta_path: &meta_path,
            armed: true,
        };
        guard.disarm();
    }

    assert!(active_client.lock().unwrap().is_some());
    assert_eq!(meta_state.lock().unwrap().attached_client_pid, Some(4321));
}

#[test]
fn attach_state_guard_drop_when_armed_clears_client_and_persists_meta() {
    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("m.json");
    let meta_initial = fixture_meta(Some(4321));
    session::write_meta_atomic(&meta_path, &meta_initial).unwrap();

    let (stream_a, _stream_b) = UnixStream::pair().unwrap();
    let client = Arc::new(Mutex::new(stream_a));
    let active_client: Mutex<Option<Arc<Mutex<UnixStream>>>> =
        Mutex::new(Some(Arc::clone(&client)));
    let meta_state = Mutex::new(meta_initial);

    drop(AttachStateGuard {
        active_client: &active_client,
        meta: &meta_state,
        meta_path: &meta_path,
        armed: true,
    });

    assert!(active_client.lock().unwrap().is_none());
    assert_eq!(meta_state.lock().unwrap().attached_client_pid, None);
    let persisted = session::read_meta(&meta_path).unwrap();
    assert_eq!(persisted.attached_client_pid, None);
}
