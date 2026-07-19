mod common;

use std::os::unix::net::UnixStream;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use skeeper::ipc::{self, ClientMsg, ServerMsg};
use skeeper::session;

use common::{READ_TIMEOUT, handshake, handshake_as, spawn_server};

#[test]
fn client_handshake_succeeds() -> Result<()> {
    let server = spawn_server("test-handshake", "/bin/cat")?;
    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut stream)?;
    Ok(())
}

#[test]
fn meta_shows_attached_after_handshake() -> Result<()> {
    let server = spawn_server("test-meta-attach", "/bin/cat")?;

    // 初期状態: 未接続
    let before = session::read_meta(&server.meta_path())?;
    assert!(before.attached_client_pids.is_empty());
    assert_eq!(before.last_attached_at, None);

    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut stream)?;

    // meta atomic write完了まで少し待つ
    sleep(Duration::from_millis(200));

    // handshake後: attached_client_pidsとlast_attached_atが埋まっている
    let after = session::read_meta(&server.meta_path())?;
    assert_eq!(
        after.attached_client_pids,
        vec![std::process::id()],
        "attached_client_pids should contain our process id after handshake"
    );
    assert!(
        after.last_attached_at.is_some(),
        "last_attached_at should be Some after handshake"
    );

    Ok(())
}

#[test]
fn multi_client_both_receive_pty_output() -> Result<()> {
    // 2 clientが同じセッションに接続し、shell (ここでは /bin/cat) にstdinを送ったら
    // 両方の client が同じStdoutを受け取れることを確認
    let server = spawn_server("test-multi-recv", "/bin/cat")?;

    // 1 テストプロセスから 2 attach を模擬するため、client_pidは意図的にずらす
    // (server側で client_pid をユニークキーにしているため)
    let pid_a = 10001u32;
    let pid_b = 10002u32;

    let mut c1 = UnixStream::connect(server.socket_path())?;
    c1.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake_as(&mut c1, pid_a)?;
    // c1の登録完了を待つ
    let start = Instant::now();
    loop {
        if let Ok(m) = session::read_meta(&server.meta_path()) {
            if m.attached_client_pids.contains(&pid_a) {
                break;
            }
        }
        if start.elapsed() >= Duration::from_secs(3) {
            bail!("c1 was not registered on meta");
        }
        sleep(Duration::from_millis(50));
    }

    let mut c2 = UnixStream::connect(server.socket_path())?;
    c2.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake_as(&mut c2, pid_b)?;
    // c2の登録完了(2件揃う)を待つ
    let start = Instant::now();
    loop {
        if let Ok(m) = session::read_meta(&server.meta_path()) {
            if m.attached_client_pids.contains(&pid_a) && m.attached_client_pids.contains(&pid_b) {
                break;
            }
        }
        if start.elapsed() >= Duration::from_secs(3) {
            bail!("c2 was not registered on meta");
        }
        sleep(Duration::from_millis(50));
    }

    // c1から入力 → catがptyに折り返す → 両方に配信されるはず
    ipc::write_client_msg(&mut c1, &ClientMsg::Stdin(b"fanout-msg\n".to_vec()))?;

    let mut c1_saw = false;
    for _ in 0..40 {
        if let Ok(ServerMsg::Stdout(bytes)) = ipc::read_server_msg(&mut c1) {
            if bytes.windows(10).any(|w| w == b"fanout-msg") {
                c1_saw = true;
                break;
            }
        }
    }
    assert!(c1_saw, "c1 should receive fanout-msg from pty");

    let mut c2_saw = false;
    for _ in 0..40 {
        if let Ok(ServerMsg::Stdout(bytes)) = ipc::read_server_msg(&mut c2) {
            if bytes.windows(10).any(|w| w == b"fanout-msg") {
                c2_saw = true;
                break;
            }
        }
    }
    assert!(c2_saw, "c2 should also receive fanout-msg from pty");

    Ok(())
}

#[test]
fn individual_detach_via_client_close() -> Result<()> {
    // 2 clientが接続 → 1台目のsocketをclose → server側でmetaから1台目のpidが外れる
    // 2台目は影響を受けずattached状態のまま
    let server = spawn_server("test-indiv-detach", "/bin/cat")?;

    let pid_a = 20001u32;
    let pid_b = 20002u32;

    let mut c1 = UnixStream::connect(server.socket_path())?;
    c1.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake_as(&mut c1, pid_a)?;

    let mut c2 = UnixStream::connect(server.socket_path())?;
    c2.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake_as(&mut c2, pid_b)?;

    // 2台とも登録されるまで待つ
    let start = Instant::now();
    loop {
        if let Ok(m) = session::read_meta(&server.meta_path()) {
            if m.attached_client_pids.contains(&pid_a) && m.attached_client_pids.contains(&pid_b) {
                break;
            }
        }
        if start.elapsed() >= Duration::from_secs(3) {
            bail!("two clients were not both registered on meta");
        }
        sleep(Duration::from_millis(50));
    }

    // c1のsocketをshutdown(=EOFにする)
    c1.shutdown(std::net::Shutdown::Both)?;
    drop(c1);

    // meta.attached_client_pidsからpid_aだけが外れるのを待つ
    let start = Instant::now();
    let mut ok = false;
    while start.elapsed() < Duration::from_secs(3) {
        if let Ok(m) = session::read_meta(&server.meta_path()) {
            if !m.attached_client_pids.contains(&pid_a) && m.attached_client_pids.contains(&pid_b) {
                ok = true;
                break;
            }
        }
        sleep(Duration::from_millis(50));
    }
    assert!(
        ok,
        "expected pid_a to be removed and pid_b to remain in meta.attached_client_pids"
    );

    // c2はまだ生きている: stdinがpty経由でechoされてくる
    ipc::write_client_msg(&mut c2, &ClientMsg::Stdin(b"still-here\n".to_vec()))?;
    let mut c2_ok = false;
    for _ in 0..40 {
        if let Ok(ServerMsg::Stdout(bytes)) = ipc::read_server_msg(&mut c2) {
            if bytes.windows(10).any(|w| w == b"still-here") {
                c2_ok = true;
                break;
            }
        }
    }
    assert!(c2_ok, "c2 should still be attached and echoing");

    Ok(())
}
