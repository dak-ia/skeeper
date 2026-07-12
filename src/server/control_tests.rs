use super::*;

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use tempfile::tempdir;
use time::macros::datetime;
use uuid::Uuid;

use crate::ipc::{ControlMsg, write_control_msg};
use crate::server::DETACH_REQUESTED;
use crate::session::{self, SessionMeta};

// DETACH_REQUESTEDはprocess-globalなstaticなので、
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
        attached_client_pid: None,
    }
}

/// UnixStream::pairで直結した2本のstreamのうち片方にmsgを書き、
/// 反対側をhandle_control_messageに食わせる。listener/threadを使わない同期テスト構成
fn feed_message(msg: &ControlMsg, meta: &Mutex<SessionMeta>, meta_path: &std::path::Path) {
    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    write_control_msg(&mut client_side, msg).unwrap();
    // client_sideを閉じておくとread_control_msg内のread_exactが確定的にEOFで抜ける(ここでは1メッセージのみ検証)
    drop(client_side);
    handle_control_message(&mut server_side, meta, meta_path);
}

#[test]
fn detach_request_sets_flag() {
    let _guard = LOCK.lock().unwrap();
    DETACH_REQUESTED.store(false, Ordering::SeqCst);

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();

    feed_message(&ControlMsg::RequestDetach, &meta_state, &meta_path);

    let observed = DETACH_REQUESTED.load(Ordering::SeqCst);
    // panicで抜けても他テストにフラグを持ち越さないよう、assertより先に戻す
    DETACH_REQUESTED.store(false, Ordering::SeqCst);
    assert!(observed);
}

#[test]
fn rename_request_updates_meta_and_persists() {
    let _guard = LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());
    session::write_meta_atomic(&meta_path, &meta_state.lock().unwrap()).unwrap();

    feed_message(
        &ControlMsg::RequestRename {
            new_name: "renamed".to_string(),
        },
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
    DETACH_REQUESTED.store(false, Ordering::SeqCst);

    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("meta.json");
    let meta_state = Mutex::new(dummy_meta());

    let (mut client_side, mut server_side) = UnixStream::pair().unwrap();
    // 意図的に途中で切って read_exact を EOF で失敗させる
    let _ = client_side.write_all(&[0u8, 0u8]);
    drop(client_side);
    handle_control_message(&mut server_side, &meta_state, &meta_path);

    assert!(!DETACH_REQUESTED.load(Ordering::SeqCst));
    assert_eq!(meta_state.lock().unwrap().name, "orig");
}
