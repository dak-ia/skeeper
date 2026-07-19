use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;
use portable_pty::{MasterPty, PtySize};
use time::OffsetDateTime;

use crate::ipc::{self, ClientMsg, ServerMsg};
use crate::session::{self, SessionMeta};

use super::guards::AttachStateGuard;
use super::{
    ClientEvent, ClientHandle, HANDSHAKE_READ_TIMEOUT, HandleOutcome, LAST_STDIN_CLIENT,
    POLL_INTERVAL, SOCKET_WRITE_TIMEOUT, TERM_REQUESTED,
};

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn handle_client(
    stream: UnixStream,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    active_clients: &Mutex<HashMap<u32, ClientHandle>>,
    scrollback: &Mutex<VecDeque<u8>>,
    meta: &Mutex<SessionMeta>,
    meta_path: &Path,
    child_exited: &AtomicBool,
    child_status: &Mutex<Option<i32>>,
) -> Result<HandleOutcome> {
    // 3つのhandleを用意:
    //   read_stream    : client→server読取り(handshake + attach内reader thread)
    //   shutdown_handle: Mutex外からshutdownするための独立clone(deadlock回避のキモ)
    //   write_stream   : 書き込み共有(Arc<Mutex<>>でsnapshot再生/PtyChunk配送/最終メッセージ全部から使う)
    let mut read_stream = stream.try_clone()?;
    let shutdown_handle = stream.try_clone()?;
    stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))?;
    let write_stream = Arc::new(Mutex::new(stream));

    // ---- ここからhandshake ----
    read_stream.set_nonblocking(false)?;
    read_stream.set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT))?;

    let Ok(hello) = ipc::read_client_msg(&mut read_stream) else {
        let _ = shutdown_handle.shutdown(Shutdown::Both);
        return Ok(HandleOutcome::Disconnected);
    };

    // プロトコル違反(最初のメッセージがHelloではない)は無言でshutdownする
    let ClientMsg::Hello {
        client_pid,
        cols,
        rows,
    } = hello
    else {
        let _ = shutdown_handle.shutdown(Shutdown::Both);
        return Ok(HandleOutcome::Disconnected);
    };

    // 初期端末サイズを反映(clientが接続する前に済ませる)
    let _ = master.lock().unwrap().resize(PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    });

    // ---- 順序の要: HelloOkを送信してからactive_clientsに登録する ----
    //   ・登録前に送るので、pty_readerがStdoutを差し込む余地がない
    //   ・HelloOk送信失敗ならmeta/active_clientsは触らずreturn(状態リーク無し)
    let (session_id, name) = {
        let m = meta.lock().unwrap();
        (m.id, m.name.clone())
    };
    if ipc::write_server_msg(
        &mut *write_stream.lock().unwrap(),
        &ServerMsg::HelloOk { session_id, name },
    )
    .is_err()
    {
        let _ = shutdown_handle.shutdown(Shutdown::Both);
        return Ok(HandleOutcome::Disconnected);
    }

    let should_detach = Arc::new(AtomicBool::new(false));
    // client読み取り・pty出力の両方をこのchannelに集めてattached_loopで処理する
    let (event_tx, event_rx) = mpsc::channel::<ClientEvent>();

    // ---- scrollback snapshot + active_clients登録(atomic) ----
    // scrollback + active_clientsの両ロック中に「snapshotコピー + self挿入」を一気に済ませる。
    // ロック解放後の実際のsocket書き込みは外に出しているので、遅いclientでも
    // active_clientsとscrollbackは長時間ブロックしない
    let snapshot: Vec<u8> = {
        let sb = scrollback.lock().unwrap();
        let mut acl = active_clients.lock().unwrap();
        let snap: Vec<u8> = sb.iter().copied().collect();
        acl.insert(
            client_pid,
            ClientHandle {
                should_detach: Arc::clone(&should_detach),
                event_tx: event_tx.clone(),
            },
        );
        snap
    };

    // ---- メタ更新(以降の早期returnはguardが後始末) ----
    {
        let mut m = meta.lock().unwrap();
        if !m.attached_client_pids.contains(&client_pid) {
            m.attached_client_pids.push(client_pid);
        }
        m.last_attached_at = Some(OffsetDateTime::now_utc());
        let _ = session::write_meta_atomic(meta_path, &m);
    }
    let mut attach_guard = AttachStateGuard {
        client_pid,
        active_clients,
        meta,
        meta_path,
        armed: true,
    };

    // ---- snapshot送信(両ロック外) ----
    // ロック解放後にpty_readerが送ってくるPtyChunkはevent_rxに積まれる。
    // ここでsnapshotをまず送ってから下のloopに入るので、client視点の順序は
    // snapshot → PtyChunk...となり、シェル出力の時系列が保たれる
    if !snapshot.is_empty() {
        let send_res = ipc::write_server_msg(
            &mut *write_stream.lock().unwrap(),
            &ServerMsg::Stdout(snapshot),
        );
        if send_res.is_err() {
            // guardがactive_clients/metaを掃除する
            let _ = shutdown_handle.shutdown(Shutdown::Both);
            return Ok(HandleOutcome::Disconnected);
        }
    }

    // ---- attach中の読み取りは別スレッド、attached_loopがeventを一元処理する ----
    read_stream.set_read_timeout(None)?;
    let reader_tx = event_tx.clone();
    let reader_handle = thread::spawn(move || {
        let mut r = read_stream;
        loop {
            match ipc::read_client_msg(&mut r) {
                Ok(m) => {
                    if reader_tx.send(ClientEvent::ClientMsg(Ok(m))).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = reader_tx.send(ClientEvent::ClientMsg(Err(e)));
                    break;
                }
            }
        }
    });

    let outcome = attached_loop(
        writer,
        master,
        &write_stream,
        &event_rx,
        child_exited,
        &should_detach,
        client_pid,
    );

    // ---- 順序の要: 先にactive_clientsから外してpty_readerのfanout先を絞る ----
    // pty_readerはacl lockを取ってsend、send失敗pidは事後掃除の設計なので、
    // ここでlockを取れた時点で「in-flight sendは既に終わっている」
    // 以降のpty_reader iterationは自clientをmapに見つけずPtyChunkを送らない
    attach_guard.disarm();
    {
        let mut acl = active_clients.lock().unwrap();
        acl.remove(&client_pid);
    }
    {
        let mut m = meta.lock().unwrap();
        m.attached_client_pids.retain(|&p| p != client_pid);
        let _ = session::write_meta_atomic(meta_path, &m);
    }

    // ---- 最後にDetachAck / SessionEndedを送る(絶対に最後のメッセージになる) ----
    let final_msg = match outcome {
        HandleOutcome::Detached => Some(ServerMsg::DetachAck),
        HandleOutcome::ChildExited => Some(ServerMsg::SessionEnded {
            exit_status: *child_status.lock().unwrap(),
        }),
        HandleOutcome::Disconnected => None,
    };
    if let Some(msg) = final_msg {
        let _ = ipc::write_server_msg(&mut *write_stream.lock().unwrap(), &msg);
    }

    // Mutex外からshutdownして、reader threadを起こす(joinがdeadlockしない)
    let _ = shutdown_handle.shutdown(Shutdown::Both);
    let _ = reader_handle.join();

    Ok(outcome)
}

/// attach中のイベントdispatch。event_rxからClientMsg / PtyChunkを受けて処理する。
/// 最終メッセージ送信・active_clients解除はここではやらない
/// (handle_client側で「先に登録解除→最後にDetachAck/SessionEnded」の順序を守るため)
#[allow(clippy::too_many_arguments)]
fn attached_loop(
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    write_stream: &Arc<Mutex<UnixStream>>,
    event_rx: &mpsc::Receiver<ClientEvent>,
    child_exited: &AtomicBool,
    should_detach: &AtomicBool,
    self_pid: u32,
) -> HandleOutcome {
    loop {
        if child_exited.load(Ordering::Acquire) || TERM_REQUESTED.load(Ordering::Acquire) {
            return HandleOutcome::ChildExited;
        }
        // 制御ソケット経由のdetach要求。ここでフラグを消費して次周に持ち越さない
        if should_detach.swap(false, Ordering::AcqRel) {
            return HandleOutcome::Detached;
        }

        match event_rx.recv_timeout(POLL_INTERVAL) {
            Ok(ClientEvent::ClientMsg(Ok(ClientMsg::Stdin(bytes)))) => {
                LAST_STDIN_CLIENT.store(self_pid, Ordering::Release);
                let mut w = writer.lock().unwrap();
                if w.write_all(&bytes).is_err() {
                    return HandleOutcome::Disconnected;
                }
                let _ = w.flush();
            }
            Ok(ClientEvent::ClientMsg(Ok(ClientMsg::Resize { cols, rows }))) => {
                let _ = master.lock().unwrap().resize(PtySize {
                    cols,
                    rows,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            Ok(ClientEvent::ClientMsg(Ok(ClientMsg::Detach))) => {
                return HandleOutcome::Detached;
            }
            Ok(ClientEvent::PtyChunk(bytes)) => {
                // Arcで共有されたchunkをclient socketに書く。ここでの遅延は当該clientだけに影響し、
                // pty_readerや他clientのfanoutは詰まらない(mpsc unboundedのため)
                let mut w = write_stream.lock().unwrap();
                if ipc::write_server_msg(&mut *w, &ServerMsg::Stdout((*bytes).clone())).is_err() {
                    return HandleOutcome::Disconnected;
                }
            }
            Ok(ClientEvent::ClientMsg(Err(_))) | Err(RecvTimeoutError::Disconnected) => {
                return HandleOutcome::Disconnected;
            }
            // handshake後の予期しないHelloとタイムアウトは同じ扱い:
            // ループ先頭の子プロセス終了/detachチェックへ戻るだけ
            Ok(ClientEvent::ClientMsg(Ok(ClientMsg::Hello { .. })))
            | Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
