mod common;

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use skeeper::ipc::{self, ClientMsg, ServerMsg};
use skeeper::session;

use common::{READ_TIMEOUT, handshake_as, spawn_server};

/// clientがshell終了/socket切断を観測したものとして許容するio::Error kind。
/// timeout系(WouldBlock/TimedOut)や不明kindは「本来届くべきものが来ていない」証拠なので許容しない
fn is_expected_close(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::BrokenPipe
    )
}

/// SIGTERMハンドラは受信後にClientMsg::Detachをserverに送るのが実体なので、
/// server側のDetachAck応答経路をIPCレベルで検証する。
/// 実signal経由のe2e(openpty subprocess + kill -TERM)は別途follow-upで拡張
#[test]
fn client_detach_message_returns_detachack() -> Result<()> {
    let server = spawn_server("test-detach-ipc", "/bin/cat")?;

    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let client_pid = 20001u32;
    handshake_as(&mut stream, client_pid)?;

    // handshake成功後、meta.attached_client_pidsに載るまで待つ
    let start = Instant::now();
    loop {
        if let Ok(m) = session::read_meta(&server.meta_path())
            && m.attached_client_pids.contains(&client_pid)
        {
            break;
        }
        if start.elapsed() >= Duration::from_secs(5) {
            bail!("client was not registered on meta before Detach");
        }
        sleep(Duration::from_millis(50));
    }

    // ClientMsg::Detach を送信し、server側で DetachAck が返ることを確認
    ipc::write_client_msg(&mut stream, &ClientMsg::Detach)?;

    // Detach 応答を待つ。ptyのstdoutが混ざる可能性があるので Stdout はスキップして DetachAck を探す
    let start = Instant::now();
    loop {
        match ipc::read_server_msg(&mut stream)? {
            ServerMsg::DetachAck => break,
            ServerMsg::SessionEnded { .. } => bail!("unexpected SessionEnded before DetachAck"),
            ServerMsg::Stdout(_) => {}
            other @ ServerMsg::HelloOk { .. } => {
                bail!("unexpected HelloOk after handshake: {other:?}")
            }
        }
        if start.elapsed() >= Duration::from_secs(5) {
            bail!("DetachAck not received within timeout");
        }
    }

    // detach完了後、metaから該当pidが外れているか(atomic writeは非同期なので少し待つ)
    let start = Instant::now();
    loop {
        if let Ok(m) = session::read_meta(&server.meta_path())
            && !m.attached_client_pids.contains(&client_pid)
        {
            return Ok(());
        }
        if start.elapsed() >= Duration::from_secs(3) {
            bail!("client_pid still present in meta after DetachAck");
        }
        sleep(Duration::from_millis(50));
    }
}

/// shellがexitするケースでSessionEndedが届くことを検証する。
/// `/bin/false`のような即exitでは serverがcleanup済で connect/handshake すら失敗する経路もある。
/// その場合はEOF/reset/broken pipeのみshell終了観測として許容し、timeout等は失敗扱いにする
#[test]
fn session_ended_or_conn_reset_when_shell_exits() -> Result<()> {
    let server = spawn_server("test-session-ended", "/bin/false")?;

    let Ok(mut stream) = UnixStream::connect(server.socket_path()) else {
        // socketがすでに消えている → 目的の「shellが終わった」観測は達成
        return Ok(());
    };
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let client_pid = 20002u32;
    if let Err(e) = handshake_as(&mut stream, client_pid) {
        // handshake中に接続が閉じられた場合はEOF/resetなら等価扱い、それ以外は失敗
        let io_err = e.downcast_ref::<std::io::Error>().map(std::io::Error::kind);
        if let Some(kind) = io_err
            && is_expected_close(kind)
        {
            return Ok(());
        }
        bail!("handshake failed with unexpected error: {e}");
    }

    // SessionEnded が届くか、read側でEOF/reset/broken pipeならshell終了の観測として扱う
    let start = Instant::now();
    loop {
        match ipc::read_server_msg(&mut stream) {
            Ok(ServerMsg::SessionEnded { .. }) => return Ok(()),
            Ok(ServerMsg::DetachAck) => bail!("unexpected DetachAck for exiting shell"),
            Ok(ServerMsg::Stdout(_)) => {}
            Ok(other @ ServerMsg::HelloOk { .. }) => bail!("unexpected HelloOk: {other:?}"),
            Err(e) if is_expected_close(e.kind()) => return Ok(()),
            Err(e) => bail!("unexpected read error: {e} (kind={:?})", e.kind()),
        }
        if start.elapsed() >= Duration::from_secs(5) {
            bail!("neither SessionEnded nor conn-reset observed within timeout");
        }
    }
}
