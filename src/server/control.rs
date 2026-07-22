use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::ipc::{self, ControlMsg, ControlResponse, RenameResponse};
use crate::runtime_lock;
use crate::session::{self, SessionMeta};

use super::{ClientHandle, HANDSHAKE_READ_TIMEOUT, LAST_STDIN_CLIENT};

/// 制御ソケットの受付ループ。接続毎に1メッセージ受け取り必要な状態変更をしてから閉じる。
/// blocking accept、プロセス終了時にkernelがthreadを回収する前提
pub(super) fn control_listener_loop(
    listener: &UnixListener,
    active_clients: &Mutex<HashMap<u32, ClientHandle>>,
    meta: &Mutex<SessionMeta>,
    meta_path: &Path,
) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        handle_control_message(&mut stream, active_clients, meta, meta_path);
    }
}

fn handle_control_message(
    stream: &mut UnixStream,
    active_clients: &Mutex<HashMap<u32, ClientHandle>>,
    meta: &Mutex<SessionMeta>,
    meta_path: &Path,
) {
    // 接続後lenを送らない停滞クライアントで以降のacceptを止めないよう、
    // 短めのread timeoutを貼る。同じ用途でmain socket側もHANDSHAKE_READ_TIMEOUTを使っている
    let _ = stream.set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT));
    // 1メッセージだけ読んで処理する。malformed/タイムアウトはignore(悪意ある接続への防御)
    match ipc::read_control_msg(stream) {
        Ok(ControlMsg::RequestDetach) => {
            // 0=まだstdin送信なし、対象なし
            let target = LAST_STDIN_CLIENT.load(Ordering::Acquire);
            if target == 0 {
                return;
            }
            let acl = active_clients.lock().unwrap();
            if let Some(handle) = acl.get(&target) {
                handle.should_detach.store(true, Ordering::Release);
            }
        }
        Ok(ControlMsg::RequestRename { new_name }) => {
            let base_dir = meta_path
                .parent()
                .expect("meta_path is always <base_dir>/<uuid>.json");
            // lock失敗はENOSPC等のIO問題と同種なのでFailed扱い(clientに再試行を促す)
            let response = match runtime_lock::acquire_runtime_lock(base_dir) {
                Ok(_lock) => process_rename_request(&new_name, meta, meta_path, base_dir),
                Err(_) => RenameResponse::Failed,
            };
            let _ = ipc::write_control_response(stream, &ControlResponse::Rename(response));
        }
        Ok(ControlMsg::QueryCurrentClient) => {
            let pid = LAST_STDIN_CLIENT.load(Ordering::Acquire);
            // write失敗はclientが既に切ってしまった等なので無視する(list側もsilent ignore)
            let _ = ipc::write_control_response(stream, &ControlResponse::CurrentClient { pid });
        }
        Ok(ControlMsg::SwitchClient { target_socket_path }) => {
            let target = LAST_STDIN_CLIENT.load(Ordering::Acquire);
            if target == 0 {
                return;
            }
            let acl = active_clients.lock().unwrap();
            if let Some(handle) = acl.get(&target) {
                // fullで届かない場合はどうにもならないのでsilent drop(clientは動き続ける)
                let _ = handle
                    .event_tx
                    .try_send(super::ClientEvent::SwitchToRequested(target_socket_path));
            }
        }
        Err(_) => {}
    }
}

/// flock保持中の前提でrename requestを処理する。呼び出し側でlockを取ってから来る
fn process_rename_request(
    new_name: &str,
    meta: &Mutex<SessionMeta>,
    meta_path: &Path,
    base_dir: &Path,
) -> RenameResponse {
    let mut m = meta.lock().unwrap();
    if m.name == new_name {
        return RenameResponse::Unchanged;
    }
    // 自meta以外で同名を使っているsessionがあればConflict
    let taken = session::list_all_meta(base_dir).unwrap_or_default();
    if taken
        .iter()
        .any(|other| other.id != m.id && other.name == new_name)
    {
        return RenameResponse::Conflict;
    }
    // Mutex保持中に完結させて他threadに中間状態を見せない
    let old_name = m.name.clone();
    m.name = new_name.to_string();
    if session::write_meta_atomic(meta_path, &m).is_err() {
        m.name = old_name;
        return RenameResponse::Failed;
    }
    RenameResponse::Ok
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
