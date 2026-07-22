use super::{AttachStateGuard, ClientHandle, SessionFileGuard};
use crate::server::ClientEvent;
use crate::session::{self, SessionMeta};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use time::macros::datetime;
use uuid::Uuid;

fn fixture_meta(attached_pids: Vec<u32>) -> SessionMeta {
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
        attached_client_pids: attached_pids,
    }
}

/// Receiverはテスト終了までscopeで保持しないとevent_tx.sendがErrになる。
/// このtestではsendしないので実害はないが、Receiverの生存を明示する意図で一緒に返す
fn make_client_handle() -> (ClientHandle, mpsc::Receiver<ClientEvent>) {
    let (event_tx, event_rx) = mpsc::sync_channel::<ClientEvent>(16);
    let handle = ClientHandle {
        attach_id: 1,
        cols: 80,
        rows: 24,
        should_detach: Arc::new(AtomicBool::new(false)),
        event_tx,
    };
    (handle, event_rx)
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
    let meta_initial = fixture_meta(vec![4321]);
    session::write_meta_atomic(&meta_path, &meta_initial).unwrap();

    let (handle, _peer) = make_client_handle();
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(4321, handle);
    let active_clients = Mutex::new(map);
    let meta_state = Mutex::new(meta_initial);

    {
        let mut guard = AttachStateGuard {
            client_pid: 4321,
            attach_id: 1,
            active_clients: &active_clients,
            meta: &meta_state,
            meta_path: &meta_path,
            master: None,
            armed: true,
        };
        guard.disarm();
    }

    assert!(active_clients.lock().unwrap().contains_key(&4321));
    assert_eq!(meta_state.lock().unwrap().attached_client_pids, vec![4321]);
}

#[test]
fn attach_state_guard_drop_when_armed_clears_client_and_persists_meta() {
    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("m.json");
    let meta_initial = fixture_meta(vec![4321]);
    session::write_meta_atomic(&meta_path, &meta_initial).unwrap();

    let (handle, _peer) = make_client_handle();
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(4321, handle);
    let active_clients = Mutex::new(map);
    let meta_state = Mutex::new(meta_initial);

    drop(AttachStateGuard {
        client_pid: 4321,
        attach_id: 1,
        active_clients: &active_clients,
        meta: &meta_state,
        meta_path: &meta_path,
        master: None,
        armed: true,
    });

    assert!(!active_clients.lock().unwrap().contains_key(&4321));
    assert!(meta_state.lock().unwrap().attached_client_pids.is_empty());
    let persisted = session::read_meta(&meta_path).unwrap();
    assert!(persisted.attached_client_pids.is_empty());
}

#[test]
fn attach_state_guard_drop_skips_when_attach_id_mismatched() {
    // 同一pidで新しいattachに置換された(attach_idが更新済み)状況を再現。
    // 古い側のguard dropはslotを他者所有と判定してmap/metaを触らないべき
    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("m.json");
    let meta_initial = fixture_meta(vec![7777]);
    session::write_meta_atomic(&meta_path, &meta_initial).unwrap();

    let (new_handle, _peer) = make_client_handle();
    // 新attachはattach_id=2、guardは古いattach_id=1を持って落ちる
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(
        7777,
        ClientHandle {
            attach_id: 2,
            ..new_handle
        },
    );
    let active_clients = Mutex::new(map);
    let meta_state = Mutex::new(meta_initial);

    drop(AttachStateGuard {
        client_pid: 7777,
        attach_id: 1,
        active_clients: &active_clients,
        meta: &meta_state,
        meta_path: &meta_path,
        master: None,
        armed: true,
    });

    // slotは新attach所有のまま残り、meta_pidsも削除されない
    assert!(active_clients.lock().unwrap().contains_key(&7777));
    assert_eq!(
        meta_state.lock().unwrap().attached_client_pids,
        vec![7777],
        "attach_id不一致でmeta_pidsも変更されないこと"
    );
}

#[test]
fn attach_state_guard_drop_only_removes_own_pid() {
    // 複数clientが登録された状態で、自分のpidだけを外し他のclientはmap/metaに残す
    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("m.json");
    let meta_initial = fixture_meta(vec![100, 200]);
    session::write_meta_atomic(&meta_path, &meta_initial).unwrap();

    let (h100, _p100) = make_client_handle();
    let (h200, _p200) = make_client_handle();
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(100, h100);
    map.insert(200, h200);
    let active_clients = Mutex::new(map);
    let meta_state = Mutex::new(meta_initial);

    drop(AttachStateGuard {
        client_pid: 100,
        attach_id: 1,
        active_clients: &active_clients,
        meta: &meta_state,
        meta_path: &meta_path,
        master: None,
        armed: true,
    });

    let acl = active_clients.lock().unwrap();
    assert!(!acl.contains_key(&100));
    assert!(acl.contains_key(&200));
    drop(acl);
    assert_eq!(meta_state.lock().unwrap().attached_client_pids, vec![200]);
}
