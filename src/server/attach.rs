use std::collections::VecDeque;
use std::io::{self, Write};
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

use crate::ipc::{self, ClientMsg, HelloErrorReason, ServerMsg};
use crate::session::{self, SessionMeta};

use super::guards::AttachStateGuard;
use super::{
    DETACH_REQUESTED, HANDSHAKE_READ_TIMEOUT, HandleOutcome, POLL_INTERVAL, SOCKET_WRITE_TIMEOUT,
    TERM_REQUESTED,
};

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn handle_client(
    stream: UnixStream,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    writer: &mut Box<dyn Write + Send>,
    active_client: &Mutex<Option<Arc<Mutex<UnixStream>>>>,
    scrollback: &Mutex<VecDeque<u8>>,
    meta: &Mutex<SessionMeta>,
    meta_path: &Path,
    child_exited: &AtomicBool,
    child_status: &Mutex<Option<i32>>,
) -> Result<HandleOutcome> {
    // 3つのhandleを用意:
    //   read_stream    : クライアント→サーバの読取り(handshake + attach内reader thread)
    //   shutdown_handle: Mutex外からshutdownするための独立clone(deadlock回避のキモ)
    //   write_stream   : 書き込み共有(Arc<Mutex<>>でpty_readerとmainから)
    let mut read_stream = stream.try_clone()?;
    let shutdown_handle = stream.try_clone()?;
    stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))?;
    let write_stream = Arc::new(Mutex::new(stream));

    // ---- ここからhandshake ----
    read_stream.set_nonblocking(false)?;
    read_stream.set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT))?;

    let Ok(hello) = ipc::read_client_msg(&mut read_stream) else {
        let _ = shutdown_handle.shutdown(Shutdown::Both);
        return Ok(HandleOutcome::Rejected);
    };

    // プロトコル違反(最初のメッセージがHelloではない)は、read失敗時と同様に無言でshutdownする。
    // HelloErrorReasonにAlreadyAttached以外のvariantを追加してもクライアントに実装詳細を出すだけなので、
    // 接続破棄で「異常なクライアント」を明確に切り離す方が扱いやすい
    let ClientMsg::Hello {
        client_pid,
        cols,
        rows,
    } = hello
    else {
        let _ = shutdown_handle.shutdown(Shutdown::Both);
        return Ok(HandleOutcome::Rejected);
    };

    // 既に別クライアントが接続していれば拒否
    let already_attached = { meta.lock().unwrap().attached_client_pid.is_some() };
    if already_attached {
        let _ = ipc::write_server_msg(
            &mut *write_stream.lock().unwrap(),
            &ServerMsg::HelloError(HelloErrorReason::AlreadyAttached),
        );
        let _ = shutdown_handle.shutdown(Shutdown::Both);
        return Ok(HandleOutcome::Rejected);
    }

    // 初期端末サイズを反映(clientが接続する前に済ませる)
    let _ = master.lock().unwrap().resize(PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    });

    // ---- 順序の要: HelloOkを送信してからactive_clientを登録する ----
    //   ・登録前に送るので、pty_readerがStdoutを差し込む余地がない
    //   ・HelloOk送信失敗ならmeta/active_clientは触らずreturn(状態リーク無し)
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

    // ---- スクロールバック再生 + active_client登録(atomic) ----
    // scrollback + active_clientの両ロックを同時に握ることで:
    //   ・pty_reader_loopは(同じ順序で)ロック取得待ちで進めない
    //   ・「scrollback読み出し → クライアントに送信 → active_client登録」を割り込みなしで一連に行える
    // 副作用: 再生中(数十ms)はpty出力がkernel bufferで詰まるが、shellが一時的に停滞するだけで実害なし
    {
        let sb = scrollback.lock().unwrap();
        let mut acl = active_client.lock().unwrap();

        if !sb.is_empty() {
            let bytes: Vec<u8> = sb.iter().copied().collect();
            let send_res = {
                let mut w = write_stream.lock().unwrap();
                ipc::write_server_msg(&mut *w, &ServerMsg::Stdout(bytes))
            };
            if send_res.is_err() {
                drop(acl);
                drop(sb);
                let _ = shutdown_handle.shutdown(Shutdown::Both);
                return Ok(HandleOutcome::Disconnected);
            }
        }

        // meta.attached_client_pid=Someを書く前にフラグをクリアする。
        // クリア後の書き込みで、client側の「メタでattached確認 → ctl送信」経路のフラグは
        // このattachに対する信号として取りこぼしなく検知される
        DETACH_REQUESTED.store(false, Ordering::Release);
        *acl = Some(Arc::clone(&write_stream));
    }

    // ---- メタ更新(以降の早期returnはguardが後始末) ----
    {
        let mut m = meta.lock().unwrap();
        m.attached_client_pid = Some(client_pid);
        m.last_attached_at = Some(OffsetDateTime::now_utc());
        let _ = session::write_meta_atomic(meta_path, &m);
    }
    let mut attach_guard = AttachStateGuard {
        active_client,
        meta,
        meta_path,
        armed: true,
    };

    // ---- attach中の読み取りは別スレッド、mainはmpscで受ける ----
    read_stream.set_read_timeout(None)?;
    let (msg_tx, msg_rx) = mpsc::channel::<io::Result<ClientMsg>>();
    let reader_handle = thread::spawn(move || {
        let mut r = read_stream;
        loop {
            match ipc::read_client_msg(&mut r) {
                Ok(m) => {
                    if msg_tx.send(Ok(m)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = msg_tx.send(Err(e));
                    break;
                }
            }
        }
    });

    let outcome = attached_loop(writer, master, &msg_rx, child_exited);

    // ---- 順序の要: 先にactive_client=Noneにしてin-flight pty_reader書き込みを絞る ----
    // pty_reader_loopはouter lockを保持したまま書くので、
    // ここでlockを取れる時点で「in-flight書き込みは既に終わっている」
    // 以降のpty_reader iterationはNoneを見てStdoutを送らなくなる
    attach_guard.disarm();
    *active_client.lock().unwrap() = None;
    {
        let mut m = meta.lock().unwrap();
        m.attached_client_pid = None;
        let _ = session::write_meta_atomic(meta_path, &m);
    }

    // ---- 最後にDetachAck / SessionEndedを送る(絶対に最後のメッセージになる) ----
    let final_msg = match outcome {
        HandleOutcome::Detached => Some(ServerMsg::DetachAck),
        HandleOutcome::ChildExited => Some(ServerMsg::SessionEnded {
            exit_status: *child_status.lock().unwrap(),
        }),
        HandleOutcome::Disconnected | HandleOutcome::Rejected => None,
    };
    if let Some(msg) = final_msg {
        let _ = ipc::write_server_msg(&mut *write_stream.lock().unwrap(), &msg);
    }

    // Mutex外からshutdownして、reader threadを起こす(joinがdeadlockしない)
    let _ = shutdown_handle.shutdown(Shutdown::Both);
    let _ = reader_handle.join();

    Ok(outcome)
}

/// attach中のイベントdispatch。最終メッセージ送信・active_client解除はここではやらない
/// (handle_client側で「先に登録解除→最後にDetachAck/SessionEnded」の順序を守るため)
fn attached_loop(
    writer: &mut Box<dyn Write + Send>,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    msg_rx: &mpsc::Receiver<io::Result<ClientMsg>>,
    child_exited: &AtomicBool,
) -> HandleOutcome {
    loop {
        if child_exited.load(Ordering::Acquire) || TERM_REQUESTED.load(Ordering::Acquire) {
            return HandleOutcome::ChildExited;
        }
        // 制御ソケット経由のdetach要求。swapでフラグを消費して次周に持ち越さない
        if DETACH_REQUESTED.swap(false, Ordering::AcqRel) {
            return HandleOutcome::Detached;
        }

        match msg_rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(ClientMsg::Stdin(bytes))) => {
                if writer.write_all(&bytes).is_err() {
                    return HandleOutcome::Disconnected;
                }
                let _ = writer.flush();
            }
            Ok(Ok(ClientMsg::Resize { cols, rows })) => {
                let _ = master.lock().unwrap().resize(PtySize {
                    cols,
                    rows,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            Ok(Ok(ClientMsg::Detach)) => {
                return HandleOutcome::Detached;
            }
            Ok(Ok(ClientMsg::Hello { .. })) | Err(RecvTimeoutError::Timeout) => {
                // handshake後の予期しないHelloは無視、
                // タイムアウトはループ先頭の子プロセス終了チェックへ戻るだけ
            }
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                return HandleOutcome::Disconnected;
            }
        }
    }
}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
