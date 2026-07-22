use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use crossterm::terminal;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

use crate::ipc::{self, ClientMsg, ServerMsg};
use crate::term_guard::TerminalGuard;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const STDIN_BUF_SIZE: usize = 4096;
/// server側でqueue Full切断された/serverが一時的に応答不能になった際、
/// clientが試みる再接続回数の上限。slowな端末で数回kickされる程度は救い、
/// 恒常的なserver死亡はここで諦めてexit
const MAX_REATTACH_ATTEMPTS: u8 = 5;
/// 再接続の間隔。詰めすぎるとspin、開けすぎるとUXが崩れる
const REATTACH_BACKOFF: Duration = Duration::from_millis(200);

/// stdin forward threadが書き込むUnixStream slot。attach外のouter loopが
/// 再attach/session切替のたびにswapする。stdinはCloseできないので単一threadを
/// プロセス寿命に紐付けて生存させ、書き込み先だけを差し替える設計
type WriteSlot = Arc<Mutex<Option<Arc<Mutex<UnixStream>>>>>;

/// SIGWINCHを受けたら立てるフラグ。attachメインループが検知してResize送信する
static SIGWINCH_RECEIVED: AtomicBool = AtomicBool::new(false);

/// SIGTERMを受けたら立てるフラグ。attachメインループが検知してDetach送信する
static SIGTERM_RECEIVED: AtomicBool = AtomicBool::new(false);

extern "C" fn sigwinch_handler(_: nix::libc::c_int) {
    SIGWINCH_RECEIVED.store(true, Ordering::SeqCst);
}

extern "C" fn sigterm_handler(_: nix::libc::c_int) {
    SIGTERM_RECEIVED.store(true, Ordering::SeqCst);
}

fn install_sigwinch_handler() -> Result<()> {
    // SA_RESTARTで、SIGWINCH受信時にstdin.readなどのブロッキングI/OがEINTRで死なないようにする
    // (フラグはメインループがpollで検知する)
    let action = SigAction::new(
        SigHandler::Handler(sigwinch_handler),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    unsafe { sigaction(Signal::SIGWINCH, &action) }
        .context("Failed to install SIGWINCH handler")?;
    Ok(())
}

fn install_sigterm_handler() -> Result<()> {
    // 外部プロセスからの`kill -TERM <client_pid>`を、terminal閉じ(SIGHUP)相当の
    // 「このattachだけ穏やかにdetach」として扱う。プロセス即死ではなくフラグを立てて、
    // ループがDetachをserverに送る。SA_RESTARTはSIGWINCHと同じ理由
    let action = SigAction::new(
        SigHandler::Handler(sigterm_handler),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    unsafe { sigaction(Signal::SIGTERM, &action) }.context("Failed to install SIGTERM handler")?;
    Ok(())
}

/// attach_onceの終了理由
enum AttachOutcome {
    /// server側が明示的にsession終了/detachを通知した(SessionEnded / DetachAck)。retryしない
    Terminated,
    /// socketが予期せず閉じた(Full切断・server crash・network reset)。retry対象
    UnexpectedClose,
    /// serverから別socketへの切り替え指示(SwitchTo)。次回iterationはそのpathで接続
    SwitchTo(PathBuf),
}

pub fn attach(socket_path: &Path) -> Result<()> {
    install_sigwinch_handler()?;
    install_sigterm_handler()?;

    // TerminalGuardはretry中もrawモード維持のためouter loopで持つが、
    // 最初のconnect失敗を"socket path付きerr"で返せるように、
    // 初回接続成功が確認できてから初めてenterする(以降retry中はSomeのまま)
    let mut term_guard: Option<TerminalGuard> = None;
    let mut current: PathBuf = socket_path.to_path_buf();
    let mut retry_budget: u8 = 0;
    let mut ever_connected = false;

    // stdin threadはプロセス生存中1本だけ(reattach/session切替でも作り直さない)。
    // 書き込み先のUnixStreamはWriteSlotで差し替える
    let write_slot: WriteSlot = Arc::new(Mutex::new(None));
    let stdin_thread_started = std::sync::Once::new();

    loop {
        let stream = match connect_and_handshake(&current) {
            Ok(s) => {
                ever_connected = true;
                s
            }
            // 初回未接続でのconnect失敗はretryせず即返す(server dead等の恒常的問題)
            Err(e) if !ever_connected => return Err(e),
            Err(_) => {
                retry_budget += 1;
                if retry_budget >= MAX_REATTACH_ATTEMPTS {
                    bail!(
                        "Server connection lost after {MAX_REATTACH_ATTEMPTS} reconnect attempts"
                    );
                }
                thread::sleep(REATTACH_BACKOFF);
                continue;
            }
        };
        if term_guard.is_none() {
            term_guard = Some(TerminalGuard::enter()?);
        }
        let write_stream = Arc::new(Mutex::new(stream.try_clone()?));
        // 単一のstdin threadを初回に1回だけspawn。以降のiterationはslot swap経由で
        // 新しいwrite_streamに向く
        stdin_thread_started.call_once(|| {
            let slot = Arc::clone(&write_slot);
            thread::spawn(move || stdin_forward_loop(&slot));
        });
        *write_slot.lock().unwrap() = Some(Arc::clone(&write_stream));

        match run_attach_session(stream, &write_stream)? {
            AttachOutcome::Terminated => return Ok(()),
            AttachOutcome::UnexpectedClose => {
                retry_budget += 1;
                if retry_budget >= MAX_REATTACH_ATTEMPTS {
                    bail!(
                        "Server connection lost after {MAX_REATTACH_ATTEMPTS} reconnect attempts"
                    );
                }
                thread::sleep(REATTACH_BACKOFF);
            }
            AttachOutcome::SwitchTo(next) => {
                // 別sessionへ乗り換え。retry counterはリセット(以降のUnexpectedCloseは別sessionのもの)
                current = next;
                retry_budget = 0;
            }
        }
    }
}

fn connect_and_handshake(socket_path: &Path) -> Result<UnixStream> {
    let mut stream = UnixStream::connect(socket_path).with_context(|| {
        format!(
            "Failed to connect to server socket {}",
            socket_path.display()
        )
    })?;

    let (cols, rows) = terminal::size().context("Failed to get terminal size")?;

    ipc::write_client_msg(
        &mut stream,
        &ClientMsg::Hello {
            client_pid: std::process::id(),
            cols,
            rows,
        },
    )?;

    let response = ipc::read_server_msg(&mut stream)?;
    match response {
        ServerMsg::HelloOk { .. } => Ok(stream),
        other => bail!("Unexpected server response: {other:?}"),
    }
}

fn run_attach_session(
    stream: UnixStream,
    write_stream: &Arc<Mutex<UnixStream>>,
) -> Result<AttachOutcome> {
    let mut server_read = stream;

    // server → mpsc thread(タイムアウト付きrecvで受けるため、直接readせずchannel経由)
    let (srv_tx, srv_rx) = mpsc::channel::<io::Result<ServerMsg>>();
    thread::spawn(move || {
        loop {
            match ipc::read_server_msg(&mut server_read) {
                Ok(m) => {
                    if srv_tx.send(Ok(m)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = srv_tx.send(Err(e));
                    break;
                }
            }
        }
    });

    let mut stdout = io::stdout();
    loop {
        if SIGWINCH_RECEIVED.swap(false, Ordering::AcqRel)
            && let Ok((c, r)) = terminal::size()
        {
            let _ = ipc::write_client_msg(
                &mut *write_stream.lock().unwrap(),
                &ClientMsg::Resize { cols: c, rows: r },
            );
        }
        // breakせずDetachだけ送る:serverがDetachAckを返した後の既存分岐で後始末する
        if SIGTERM_RECEIVED.swap(false, Ordering::AcqRel) {
            let _ = ipc::write_client_msg(&mut *write_stream.lock().unwrap(), &ClientMsg::Detach);
        }

        match srv_rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(ServerMsg::Stdout(bytes))) => {
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
            Ok(Ok(ServerMsg::SessionEnded { .. } | ServerMsg::DetachAck)) => {
                return Ok(AttachOutcome::Terminated);
            }
            Ok(Ok(ServerMsg::SwitchTo { target_socket_path })) => {
                return Ok(AttachOutcome::SwitchTo(target_socket_path));
            }
            Ok(Ok(_)) | Err(RecvTimeoutError::Timeout) => {
                // 予期しない応答は無視(ハンドシェイク後のHelloOk等)、
                // タイムアウトは次のループでのSIGWINCH/SIGTERMチェック用
            }
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                // socketが予期せず閉じた(server queue full切断・server crash等)。
                // outer loop側で再attachを試みる
                return Ok(AttachOutcome::UnexpectedClose);
            }
        }
    }
}

fn stdin_forward_loop(slot: &WriteSlot) {
    let mut stdin = io::stdin();
    let mut buf = [0u8; STDIN_BUF_SIZE];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let msg = ClientMsg::Stdin(buf[..n].to_vec());
                // 差し替え直後は必ず最新の書き込み先が入っている(swapはouter loopで完了してから
                // read解除するタイミングは制御できないが、readが返る時点でslotは最新)
                let stream_opt = slot.lock().unwrap().clone();
                if let Some(stream) = stream_opt {
                    let mut w = stream.lock().unwrap();
                    // 書込失敗は接続切れ(server側でclose)を意味するが、stdin thread自体は
                    // outerが新streamをswapしてくるので継続する。切れた側のwriteだけを捨てる
                    let _ = ipc::write_client_msg(&mut *w, &msg);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
