mod common;

use std::os::unix::net::UnixStream;
use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;

use skeeper::ipc::{self, ClientMsg, ControlMsg, ServerMsg};

use common::{READ_TIMEOUT, handshake, spawn_server};

#[test]
fn scrollback_is_replayed_on_new_client() -> Result<()> {
    // /bin/catはstdinをそのままstdoutに返すので、まず1個目のクライアントから
    // 何か入力→cat経由でstdoutに出る→サーバのscrollbackに溜まる
    // 2個目のattachでその内容がStdoutとして再生されるはず
    let server = spawn_server("test-scrollback", "/bin/cat")?;

    let mut c1 = UnixStream::connect(server.socket_path())?;
    c1.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut c1)?;
    sleep(Duration::from_millis(100));

    // stdinに投げるとcatがstdoutに折り返す
    ipc::write_client_msg(&mut c1, &ClientMsg::Stdin(b"hello-scrollback\n".to_vec()))?;

    // c1でエコーが返るのを待つ
    let mut c1_saw_echo = false;
    for _ in 0..30 {
        if let Ok(ServerMsg::Stdout(bytes)) = ipc::read_server_msg(&mut c1)
            && bytes.windows(16).any(|w| w == b"hello-scrollback")
        {
            c1_saw_echo = true;
            break;
        }
    }
    assert!(
        c1_saw_echo,
        "first client should have echoed hello-scrollback"
    );

    // c1をdetach
    let mut ctl = UnixStream::connect(server.ctl_path())?;
    ipc::write_control_msg(&mut ctl, &ControlMsg::RequestDetach)?;
    drop(ctl);
    // DetachAckまで消費
    for _ in 0..20 {
        if let Ok(ServerMsg::DetachAck) = ipc::read_server_msg(&mut c1) {
            break;
        }
    }
    drop(c1);
    sleep(Duration::from_millis(200));

    // c2で再attach → HelloOk直後にscrollbackが再生されるはず
    let mut c2 = UnixStream::connect(server.socket_path())?;
    c2.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut c2)?;

    // c2の最初のStdoutメッセージがscrollback (hello-scrollbackを含む)
    let mut c2_saw_replay = false;
    for _ in 0..10 {
        if let Ok(ServerMsg::Stdout(bytes)) = ipc::read_server_msg(&mut c2)
            && bytes.windows(16).any(|w| w == b"hello-scrollback")
        {
            c2_saw_replay = true;
            break;
        }
    }
    assert!(c2_saw_replay, "second client should see scrollback replay");
    Ok(())
}
