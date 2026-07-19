use super::*;

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use time::macros::datetime;
use uuid::Uuid;

use crate::ipc::{ControlMsg, write_control_msg};
use crate::server::{ClientEvent, ClientHandle, LAST_STDIN_CLIENT};
use crate::session::{self, SessionMeta};

// LAST_STDIN_CLIENTはprocess-globalなstaticなので、
// 同ファイル内の複数テストが並列に触ると衝突する
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn dummy_meta() -> SessionMeta {
    let ts = datetime!(2000-01-02 03:04:05 UTC);
    SessionMeta {
        id: Uuid::from_u128(0x1),
        name: "orig".to_string(),
        cwd: PathBuf::from("/tmp"),
        shell: PathBuf::from("/bin/sh"),
        created_at: ts,
        last_attached_at: None,
        server_pid: 1,
        server_started_at: ts,
        attached_client_pids: Vec::new(),
    }
}

/// テスト用に最小限のClientHandleを組み立てる。
/// Receiverはテスト内で保持しておかないとevent_tx.sendがErrになる(このtestでは
/// sendしないので実害はないが、Receiverの生存を明示する意図で一緒に返す)
fn make_client_handle() -> (ClientHandle, Arc<AtomicBool>, mpsc::Receiver<ClientEvent>) {
    let should_detach = Arc::new(AtomicBool::new(false));
    let (event_tx, event_rx) = mpsc::channel::<ClientEvent>();
    let handle = ClientHandle {
        should_detach: Arc::clone(&should_detach),
        event_tx,
    };
    (handle, should_detach, event_rx)
}

/// UnixStream::pairで直結した2本のstreamのうち片方にmsgを書き、
/// 反対側をhandle_control_messageに食わせる。listener/threadを使わない同期テスト構成
fn feed_message(
    msg: &ControlMsg,
    active_clients: &Mutex<HashMap<u32, ClientHandle>>,
    meta: &Mutex<SessionMeta>,
    meta_path: &std::path::Path,
) {
    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    write_control_msg(&mut client_side, msg).unwrap();
    // client_sideを閉じておくとread_control_msg内のread_exactが確定的にEOFで抜ける(ここでは1メッセージのみ検証)
    drop(client_side);
    handle_control_message(&mut server_side, active_clients, meta, meta_path);
}

#[test]
fn detach_request_targets_last_stdin_client() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();

    // 2 clientsをmapに登録、片方をLAST_STDIN_CLIENTに設定
    let (h_target, target_flag, _p_target) = make_client_handle();
    let (h_other, other_flag, _p_other) = make_client_handle();
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(111, h_target);
    map.insert(222, h_other);
    let active_clients = Mutex::new(map);

    LAST_STDIN_CLIENT.store(111, Ordering::SeqCst);

    feed_message(
        &ControlMsg::RequestDetach,
        &active_clients,
        &meta_state,
        &meta_path,
    );

    let target_observed = target_flag.load(Ordering::SeqCst);
    let other_observed = other_flag.load(Ordering::SeqCst);
    // panicで抜けても他テストにフラグを持ち越さないよう、assertより先に戻す
    LAST_STDIN_CLIENT.store(0, Ordering::SeqCst);
    assert!(target_observed);
    assert!(!other_observed);
}

#[test]
fn detach_request_is_noop_when_no_stdin_yet() {
    // LAST_STDIN_CLIENT=0(誰もまだstdin送っていない)ならRequestDetachは何もしない
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();

    let (handle, flag, _peer) = make_client_handle();
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(111, handle);
    let active_clients = Mutex::new(map);

    LAST_STDIN_CLIENT.store(0, Ordering::SeqCst);

    feed_message(
        &ControlMsg::RequestDetach,
        &active_clients,
        &meta_state,
        &meta_path,
    );

    let observed = flag.load(Ordering::SeqCst);
    assert!(!observed);
}

#[test]
fn detach_request_is_noop_when_target_absent() {
    // target pidがmapに無い場合(既にdetach済み)は何もしない
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();

    let (handle, flag, _peer) = make_client_handle();
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(999, handle); // 別のpid
    let active_clients = Mutex::new(map);

    LAST_STDIN_CLIENT.store(111, Ordering::SeqCst);

    feed_message(
        &ControlMsg::RequestDetach,
        &active_clients,
        &meta_state,
        &meta_path,
    );

    LAST_STDIN_CLIENT.store(0, Ordering::SeqCst);
    assert!(!flag.load(Ordering::SeqCst));
}

#[test]
fn rename_request_updates_meta_and_persists() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();

    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    feed_message(
        &ControlMsg::RequestRename {
            new_name: "renamed".to_string(),
        },
        &active_clients,
        &meta_state,
        &meta_path,
    );

    assert_eq!(meta_state.lock().unwrap().name, "renamed");
    let persisted = session::read_meta(&meta_path).unwrap();
    assert_eq!(persisted.name, "renamed");
}

#[test]
fn malformed_message_is_ignored() {
    // 悪意ある接続への防御: read_control_msgが失敗しても handle は panic せず何も変更しない
    let _guard = LOCK.lock().unwrap();
    LAST_STDIN_CLIENT.store(0, Ordering::SeqCst);

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    // 意図的に途中で切って read_exact を EOF で失敗させる
    let _ = client_side.write_all(&[0u8, 0u8]);
    drop(client_side);
    handle_control_message(&mut server_side, &active_clients, &meta_state, &meta_path);

    assert_eq!(LAST_STDIN_CLIENT.load(Ordering::SeqCst), 0);
    assert_eq!(meta_state.lock().unwrap().name, "orig");
}
