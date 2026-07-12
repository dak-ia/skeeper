use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use crate::ipc::{self, ServerMsg};

use super::{PTY_BUF_SIZE, SCROLLBACK_MAX_BYTES, Scrollback};

/// ptyのstdoutを読み続けるバックグラウンドスレッド。
/// 接続中のクライアントに対してのみStdoutを書き出し、同時にscrollback bufferにも溜める。
///
/// scrollbackへの追記からactive_clientへの書き込みまで、両ロックを保持したまま実行する。
/// 途中で新規attach側(handle_client)が同じ順で両ロックを取ろうとしても、pty_reader側の
/// 処理が終わってから割り込むため、scrollback replay直後にpty_reader由来の同一chunkが
/// 二重配信されるレースを防いでいる
#[allow(clippy::needless_pass_by_value)] // スレッド生存期間 = Arc所有期間として意図的
pub(super) fn pty_reader_loop(
    mut reader: Box<dyn Read + Send>,
    active_client: Arc<Mutex<Option<Arc<Mutex<UnixStream>>>>>,
    scrollback: Scrollback,
) {
    let mut buf = [0u8; PTY_BUF_SIZE];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut sb = scrollback.lock().unwrap();
                let space = SCROLLBACK_MAX_BYTES.saturating_sub(sb.len());
                if n > space {
                    let drop_count = n - space;
                    for _ in 0..drop_count.min(sb.len()) {
                        sb.pop_front();
                    }
                }
                sb.extend(&buf[..n]);

                // active_client登録の途中に割り込まれると、attach側がscrollback replay直後に
                // 同じchunkをpty_readerから受け取ることになる。scrollbackを持ったままaclを取る
                let guard = active_client.lock().unwrap();
                if let Some(ref s) = *guard {
                    let msg = ServerMsg::Stdout(buf[..n].to_vec());
                    if let Ok(mut w) = s.lock() {
                        let _ = ipc::write_server_msg(&mut *w, &msg);
                    }
                }
                drop(guard);
                drop(sb);
            }
        }
    }
}

#[cfg(test)]
#[path = "pty_io_tests.rs"]
mod tests;
