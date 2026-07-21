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

use crate::ipc::{
    ControlMsg, ControlResponse, RenameResponse, read_control_response, write_control_msg,
};
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
    let (event_tx, event_rx) = mpsc::sync_channel::<ClientEvent>(16);
    let handle = ClientHandle {
        attach_id: 1,
        cols: 80,
        rows: 24,
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

/// meta_pathがbase_dir配下のuuid.jsonであることをprocess_rename_requestが期待するので、
/// tempdir直下ではなくuuid.jsonの形で置く。他のsession metaを一緒に置きたいテストで使う
fn write_meta_in(base_dir: &std::path::Path, meta: &SessionMeta) -> PathBuf {
    let p = base_dir.join(format!("{}.json", meta.id));
    session::write_meta_atomic(&p, meta).unwrap();
    p
}

/// UnixStream::pair直結で1メッセージ送信+response読み取り。feed_messageと違い応答をreadできる
fn request_and_read_response(
    msg: &ControlMsg,
    active_clients: &Mutex<HashMap<u32, ClientHandle>>,
    meta: &Mutex<SessionMeta>,
    meta_path: &std::path::Path,
) -> ControlResponse {
    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    write_control_msg(&mut client_side, msg).unwrap();
    handle_control_message(&mut server_side, active_clients, meta, meta_path);
    read_control_response(&mut client_side).unwrap()
}

#[test]
fn rename_request_returns_ok_and_updates_meta() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let base_dir = dir.path();
    let mut initial = dummy_meta();
    initial.id = Uuid::from_u128(0x1);
    let meta_path = write_meta_in(base_dir, &initial);
    let meta_state = Mutex::new(initial);
    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    let resp = request_and_read_response(
        &ControlMsg::RequestRename {
            new_name: "renamed".to_string(),
        },
        &active_clients,
        &meta_state,
        &meta_path,
    );

    assert_eq!(resp, ControlResponse::Rename(RenameResponse::Ok));
    assert_eq!(meta_state.lock().unwrap().name, "renamed");
    let persisted = session::read_meta(&meta_path).unwrap();
    assert_eq!(persisted.name, "renamed");
}

#[test]
fn rename_request_with_same_name_returns_unchanged() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let base_dir = dir.path();
    let mut initial = dummy_meta();
    initial.id = Uuid::from_u128(0x2);
    initial.name = "same".to_string();
    let meta_path = write_meta_in(base_dir, &initial);
    let meta_state = Mutex::new(initial);
    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    let resp = request_and_read_response(
        &ControlMsg::RequestRename {
            new_name: "same".to_string(),
        },
        &active_clients,
        &meta_state,
        &meta_path,
    );

    assert_eq!(resp, ControlResponse::Rename(RenameResponse::Unchanged));
    // no-opなのでメモリ上のnameも変わらない
    assert_eq!(meta_state.lock().unwrap().name, "same");
}

#[test]
fn rename_request_with_existing_name_returns_conflict() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let base_dir = dir.path();

    // 別sessionが"taken"を使っている状態を作る
    let mut other = dummy_meta();
    other.id = Uuid::from_u128(0xAA);
    other.name = "taken".to_string();
    write_meta_in(base_dir, &other);

    // 自sessionは"self"、これを"taken"にrenameしようとする
    let mut self_meta = dummy_meta();
    self_meta.id = Uuid::from_u128(0xBB);
    self_meta.name = "self".to_string();
    let self_path = write_meta_in(base_dir, &self_meta);
    let meta_state = Mutex::new(self_meta);
    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    let resp = request_and_read_response(
        &ControlMsg::RequestRename {
            new_name: "taken".to_string(),
        },
        &active_clients,
        &meta_state,
        &self_path,
    );

    assert_eq!(resp, ControlResponse::Rename(RenameResponse::Conflict));
    // 自metaは変更されない
    assert_eq!(meta_state.lock().unwrap().name, "self");
    let persisted = session::read_meta(&self_path).unwrap();
    assert_eq!(persisted.name, "self");
}

#[test]
fn query_current_client_returns_last_stdin_pid() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();
    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    LAST_STDIN_CLIENT.store(4242, Ordering::SeqCst);

    // feed_messageはclient_sideをdropしてしまうので、応答を読むためここでは直に組み立てる
    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    write_control_msg(&mut client_side, &ControlMsg::QueryCurrentClient).unwrap();
    handle_control_message(&mut server_side, &active_clients, &meta_state, &meta_path);
    let resp = read_control_response(&mut client_side).unwrap();

    LAST_STDIN_CLIENT.store(0, Ordering::SeqCst);
    assert_eq!(resp, ControlResponse::CurrentClient { pid: 4242 });
}

#[test]
fn query_current_client_returns_zero_when_no_stdin_yet() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();
    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    LAST_STDIN_CLIENT.store(0, Ordering::SeqCst);

    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    write_control_msg(&mut client_side, &ControlMsg::QueryCurrentClient).unwrap();
    handle_control_message(&mut server_side, &active_clients, &meta_state, &meta_path);
    let resp = read_control_response(&mut client_side).unwrap();

    assert_eq!(resp, ControlResponse::CurrentClient { pid: 0 });
}

#[test]
fn malformed_message_is_ignored() {
    // read_control_msgが失敗してもhandleはpanicせず何も変更しない
    let _guard = LOCK.lock().unwrap();
    LAST_STDIN_CLIENT.store(0, Ordering::SeqCst);

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());

    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    // 意図的に途中で切ってread_exactをEOFで失敗させる
    let _ = client_side.write_all(&[0u8, 0u8]);
    drop(client_side);
    handle_control_message(&mut server_side, &active_clients, &meta_state, &meta_path);

    assert_eq!(LAST_STDIN_CLIENT.load(Ordering::SeqCst), 0);
    assert_eq!(meta_state.lock().unwrap().name, "orig");
}
