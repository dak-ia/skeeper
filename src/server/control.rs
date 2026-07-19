use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::ipc::{self, ControlMsg, ControlResponse};
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
            // 直近stdin送信元をdetach対象とする。0(誰もstdinを送っていない)なら何もしない
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
            let mut m = meta.lock().unwrap();
            m.name = new_name;
            let _ = session::write_meta_atomic(meta_path, &m);
        }
        Ok(ControlMsg::QueryCurrentClient) => {
            let pid = LAST_STDIN_CLIENT.load(Ordering::Acquire);
            // write失敗はclientが既に切ってしまった等なので無視する(list側もsilent ignore)
            let _ = ipc::write_control_response(stream, &ControlResponse::CurrentClient { pid });
        }
        Err(_) => {}
    }
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
