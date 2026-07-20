use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
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

pub fn attach(socket_path: &Path) -> Result<()> {
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
        ServerMsg::HelloOk { .. } => {}
        other => bail!("Unexpected server response: {other:?}"),
    }

    install_sigwinch_handler()?;
    install_sigterm_handler()?;
    let _term = TerminalGuard::enter()?;

    let write_stream = Arc::new(Mutex::new(stream.try_clone()?));
    let mut server_read = stream;

    // stdin → socket forwarding thread(プロセス終了時にkernelが片付ける)
    {
        let ws = Arc::clone(&write_stream);
        thread::spawn(move || stdin_forward_loop(&ws));
    }

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
        // SIGWINCH検知 → 新しいサイズをResizeで送る
        if SIGWINCH_RECEIVED.swap(false, Ordering::AcqRel)
            && let Ok((c, r)) = terminal::size()
        {
            let _ = ipc::write_client_msg(
                &mut *write_stream.lock().unwrap(),
                &ClientMsg::Resize { cols: c, rows: r },
            );
        }
        // SIGTERM検知 → Detachを送るだけでbreakはしない。serverがDetachAckを返し、
        // 既存のDetachAck分岐でloopを抜けて後始末する経路に合流させる
        if SIGTERM_RECEIVED.swap(false, Ordering::AcqRel) {
            let _ = ipc::write_client_msg(&mut *write_stream.lock().unwrap(), &ClientMsg::Detach);
        }

        match srv_rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(ServerMsg::Stdout(bytes))) => {
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
            Ok(Ok(ServerMsg::SessionEnded { .. } | ServerMsg::DetachAck) | Err(_))
            | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Ok(_)) | Err(RecvTimeoutError::Timeout) => {
                // 予期しない応答は無視(ハンドシェイク後のHelloOk等)、
                // タイムアウトは次のループでのSIGWINCHチェック用
            }
        }
    }

    Ok(())
}

fn stdin_forward_loop(write_stream: &Arc<Mutex<UnixStream>>) {
    let mut stdin = io::stdin();
    let mut buf = [0u8; STDIN_BUF_SIZE];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let msg = ClientMsg::Stdin(buf[..n].to_vec());
                let mut w = write_stream.lock().unwrap();
                if ipc::write_client_msg(&mut *w, &msg).is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
