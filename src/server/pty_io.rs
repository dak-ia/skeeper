use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};

use super::{ClientEvent, ClientHandle, PTY_BUF_SIZE, SCROLLBACK_MAX_BYTES, Scrollback};

/// ptyのstdoutを読み続けるバックグラウンドスレッド。
/// 接続中の全clientにfanoutと、scrollback bufferへの追記をまとめて行う。
#[allow(clippy::needless_pass_by_value)] // スレッド生存期間 = Arc所有期間として意図的
pub(super) fn pty_reader_loop(
    mut reader: Box<dyn Read + Send>,
    active_clients: Arc<Mutex<HashMap<u32, ClientHandle>>>,
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

                // fanout: 1つのVecをArcで共有し、各clientのchannelにclone senderで送る。
                // send自体は非blocking(unbounded channel)なので、1台のattached_loopが
                // 遅くても他のclientやscrollback更新はここで止まらない。
                let chunk = Arc::new(buf[..n].to_vec());
                let guard = active_clients.lock().unwrap();
                let mut failed: Vec<u32> = Vec::new();
                for (pid, handle) in guard.iter() {
                    // send失敗はattached_loopがすでに終わっている(Receiver drop)ケース。
                    // pidを覚えておいて後でmapから外し、以降のfanoutを絞る
                    if handle
                        .event_tx
                        .send(ClientEvent::PtyChunk(Arc::clone(&chunk)))
                        .is_err()
                    {
                        failed.push(*pid);
                    }
                }
                drop(guard);
                if !failed.is_empty() {
                    let mut guard = active_clients.lock().unwrap();
                    for pid in failed {
                        guard.remove(&pid);
                    }
                }
                drop(sb);
            }
        }
    }
}

#[cfg(test)]
#[path = "pty_io_tests.rs"]
mod tests;
