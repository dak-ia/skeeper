mod common;

use std::os::unix::net::UnixStream;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use skeeper::ipc::{self, ClientMsg, ControlMsg, ServerMsg};
use skeeper::session;

use common::{CLEANUP_TIMEOUT, READ_TIMEOUT, handshake, spawn_server};

#[test]
fn control_detach_sends_detach_ack() -> Result<()> {
    let server = spawn_server("test-ctldetach", "/bin/cat")?;

    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut stream)?;

    // handle_client内での登録完了待ち
    sleep(Duration::from_millis(200));

    // 制御ソケット経由のRequestDetachは「直近stdin送信元client」を対象にする実装なので、
    // 先に何か送ってLAST_STDIN_CLIENTをこのclientに固定する
    ipc::write_client_msg(&mut stream, &ClientMsg::Stdin(b"warmup\n".to_vec()))?;
    for _ in 0..30 {
        match ipc::read_server_msg(&mut stream) {
            Ok(ServerMsg::Stdout(bytes)) if bytes.windows(6).any(|w| w == b"warmup") => break,
            Ok(_) => (),
            Err(_) => break,
        }
    }

    // 制御ソケット経由でRequestDetach
    let mut ctl = UnixStream::connect(server.ctl_path())?;
    ipc::write_control_msg(&mut ctl, &ControlMsg::RequestDetach)?;
    drop(ctl);

    // DetachAckが届くはず (/bin/catは自発的にStdoutを出さないので、最初に来るサーバメッセージはDetachAck)
    // 念のため数回ループでStdoutをskip
    let mut got_ack = false;
    for _ in 0..20 {
        match ipc::read_server_msg(&mut stream) {
            Ok(ServerMsg::DetachAck) => {
                got_ack = true;
                break;
            }
            Ok(_) => (),
            Err(_) => break,
        }
    }
    assert!(got_ack, "expected DetachAck within reasonable time");

    Ok(())
}

#[test]
fn control_rename_updates_meta() -> Result<()> {
    let server = spawn_server("test-rename-before", "/bin/cat")?;

    let before = session::read_meta(&server.meta_path())?;
    assert_eq!(before.name, "test-rename-before");

    let mut ctl = UnixStream::connect(server.ctl_path())?;
    ipc::write_control_msg(
        &mut ctl,
        &ControlMsg::RequestRename {
            new_name: "test-rename-after".to_string(),
        },
    )?;
    drop(ctl);

    // metaがatomic write完了するまで待つ
    let start = Instant::now();
    while start.elapsed() < CLEANUP_TIMEOUT {
        if let Ok(meta) = session::read_meta(&server.meta_path()) {
            if meta.name == "test-rename-after" {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(50));
    }
    bail!("meta.name did not update to 'test-rename-after' within timeout")
}
