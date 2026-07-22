use super::*;

use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use portable_pty::{PtySize, native_pty_system};
use tempfile::tempdir;
use time::macros::datetime;
use uuid::Uuid;

use crate::ipc::{self, ClientMsg};
use crate::session::{self, SessionMeta};

fn fixture_meta() -> SessionMeta {
    let fixed = datetime!(2000-01-02 03:04:05 UTC);
    SessionMeta {
        id: Uuid::from_u128(0x1),
        name: "sess".to_string(),
        cwd: PathBuf::from("/"),
        shell: PathBuf::from("/bin/sh"),
        created_at: fixed,
        last_attached_at: None,
        server_pid: 1,
        server_started_at: fixed,
        schema_version: session::SCHEMA_VERSION_CURRENT,
        ipc_protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
        attached_clients: Vec::new(),
    }
}

#[test]
fn handle_client_disconnects_when_client_sends_non_hello() {
    // 最初のメッセージがHello以外(ここではDetach)なら、handle_clientは無言でshutdownし
    // HandleOutcome::Disconnectedを返す。プロトコル違反を検知した経路のsmoke test
    let (mut client, server) = UnixStream::pair().unwrap();

    // handle_client呼び出し前にkernel bufferへ書き込んでおくと、
    // handshake内のread_client_msgが即座に読み取れる
    ipc::write_client_msg(&mut client, &ClientMsg::Detach).unwrap();

    let pty = native_pty_system()
        .openpty(PtySize {
            cols: 24,
            rows: 8,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    // 拒否経路ではwriter/masterへの実書き込みは発生しないので、writerはsinkでよい
    let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(Box::new(io::sink())));
    let master: Mutex<Box<dyn MasterPty + Send>> = Mutex::new(pty.master);

    let active_clients: Mutex<HashMap<u32, ClientHandle>> = Mutex::new(HashMap::new());
    let scrollback: Mutex<VecDeque<u8>> = Mutex::new(VecDeque::new());
    let meta = Mutex::new(fixture_meta());
    let dir = tempdir().unwrap();
    let meta_path = dir.path().join("m.json");
    let child_exited = AtomicBool::new(false);
    let child_status: Mutex<Option<i32>> = Mutex::new(None);

    let outcome = handle_client(
        server,
        &master,
        &writer,
        &active_clients,
        &scrollback,
        &meta,
        &meta_path,
        &child_exited,
        &child_status,
    )
    .expect("handle_client");

    assert!(matches!(outcome, HandleOutcome::Disconnected));
    // 拒否時にサーバは状態を触らないこと(active_clients未登録、meta.attached_clientsそのまま)
    assert!(active_clients.lock().unwrap().is_empty());
    assert!(meta.lock().unwrap().attached_clients.is_empty());
    // meta_pathは書き込まれていない(HelloOk成功後にしか書かない)
    assert!(!meta_path.exists());

    // client側は接続を切られているはず。追加writeがEPIPE等で失敗することを確認
    let write_res = ipc::write_client_msg(&mut client, &ClientMsg::Detach);
    assert!(write_res.is_err());
}
