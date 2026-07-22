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
    ATTACH_ID_COUNTER, ClientEvent, ClientHandle, HANDSHAKE_READ_TIMEOUT, HandleOutcome,
    LAST_STDIN_CLIENT, POLL_INTERVAL, SOCKET_WRITE_TIMEOUT, TERM_REQUESTED, aggregate_min_size,
    attach_buffer_capacity,
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
    // shutdown_handleはMutex外からshutdownする用の独立clone(join deadlock回避)
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
    // client読み取り・pty出力の両方をこのchannelに集めてattached_loopで処理する。
    // slow client検出のためbounded sync_channel(fullでtry_send失敗→切断経路)
    let (event_tx, event_rx) = mpsc::sync_channel::<ClientEvent>(attach_buffer_capacity());
    // このattach一意のid。cleanup時にslot所有者かどうかの判定に使う(pid再利用/同一pid再attach対策)
    let attach_id = ATTACH_ID_COUNTER.fetch_add(1, Ordering::AcqRel);

    // ---- scrollback snapshot + active_clients登録 + pty size集約(atomic) ----
    let snapshot: Vec<u8> = {
        let sb = scrollback.lock().unwrap();
        let mut acl = active_clients.lock().unwrap();
        let snap: Vec<u8> = sb.iter().copied().collect();
        // 同一pidが既にmapに残っている場合、古い側にdetachシグナルを立てて置換する
        if let Some(old) = acl.remove(&client_pid) {
            old.should_detach.store(true, Ordering::Release);
        }
        acl.insert(
            client_pid,
            ClientHandle {
                attach_id,
                cols,
                rows,
                should_detach: Arc::clone(&should_detach),
                event_tx: event_tx.clone(),
            },
        );
        if let Some((c, r)) = aggregate_min_size(&acl) {
            let _ = master.lock().unwrap().resize(PtySize {
                cols: c,
                rows: r,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
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
        attach_id,
        active_clients,
        meta,
        meta_path,
        master: Some(master),
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
        active_clients,
        &write_stream,
        &event_rx,
        child_exited,
        &should_detach,
        client_pid,
    );

    // ---- 順序の要: 先にactive_clientsから外してpty_readerのfanout先を絞る ----
    attach_guard.disarm();
    let is_still_current = {
        let mut acl = active_clients.lock().unwrap();
        let owned = matches!(acl.get(&client_pid), Some(h) if h.attach_id == attach_id);
        if owned {
            acl.remove(&client_pid);
            // 抜けたので残りclientのminでptyを再集約(0 clientならkeep最後のサイズ)
            if let Some((c, r)) = aggregate_min_size(&acl) {
                let _ = master.lock().unwrap().resize(PtySize {
                    cols: c,
                    rows: r,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
        owned
    };
    if is_still_current {
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
    active_clients: &Mutex<HashMap<u32, ClientHandle>>,
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
                // 自client分を更新してから全clientのminでresize
                let mut acl = active_clients.lock().unwrap();
                if let Some(h) = acl.get_mut(&self_pid) {
                    h.cols = cols;
                    h.rows = rows;
                }
                if let Some((c, r)) = aggregate_min_size(&acl) {
                    let _ = master.lock().unwrap().resize(PtySize {
                        cols: c,
                        rows: r,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            Ok(ClientEvent::ClientMsg(Ok(ClientMsg::Detach))) => {
                return HandleOutcome::Detached;
            }
            Ok(ClientEvent::PtyChunk(bytes)) => {
                // 遅延は当該clientだけに影響(bounded sync_channelで他clientやpty_readerに波及しない)
                let mut w = write_stream.lock().unwrap();
                if ipc::write_server_msg(&mut *w, &ServerMsg::Stdout((*bytes).clone())).is_err() {
                    return HandleOutcome::Disconnected;
                }
            }
            Ok(ClientEvent::SwitchToRequested(target_socket_path)) => {
                // clientに送出後にDetachedで抜ける(client側でSwitchTo受信→再attach)
                let mut w = write_stream.lock().unwrap();
                let _ = ipc::write_server_msg(&mut *w, &ServerMsg::SwitchTo { target_socket_path });
                return HandleOutcome::Detached;
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
