use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use super::{ClientEvent, ClientHandle, PTY_BUF_SIZE, SCROLLBACK_MAX_BYTES, Scrollback};

/// ptyのstdoutを読み続けるバックグラウンドスレッド。全clientへfanoutとscrollback追記を担う
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

                // Full=slow client(should_detachで切断)、Disconnected=attached_loop終了
                let chunk = Arc::new(buf[..n].to_vec());
                let guard = active_clients.lock().unwrap();
                // 同一pidで新規attachが割り込むと別slot所有者を誤削除するのでattach_idで判定
                let mut failed: Vec<(u32, u64)> = Vec::new();
                for (pid, handle) in guard.iter() {
                    match handle
                        .event_tx
                        .try_send(ClientEvent::PtyChunk(Arc::clone(&chunk)))
                    {
                        Ok(()) => {}
                        Err(mpsc::TrySendError::Full(_)) => {
                            handle
                                .should_detach
                                .store(true, std::sync::atomic::Ordering::Release);
                            failed.push((*pid, handle.attach_id));
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => {
                            failed.push((*pid, handle.attach_id));
                        }
                    }
                }
                drop(guard);
                if !failed.is_empty() {
                    let mut guard = active_clients.lock().unwrap();
                    for (pid, expected_aid) in failed {
                        if guard.get(&pid).map(|h| h.attach_id) == Some(expected_aid) {
                            guard.remove(&pid);
                        }
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
