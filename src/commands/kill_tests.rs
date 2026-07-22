use super::*;

use time::OffsetDateTime;
use uuid::Uuid;

fn orphan_meta() -> SessionMeta {
    SessionMeta {
        id: Uuid::from_u128(0x1),
        name: "test".to_string(),
        cwd: std::path::PathBuf::from("/"),
        shell: std::path::PathBuf::from("/bin/sh"),
        created_at: OffsetDateTime::UNIX_EPOCH,
        last_attached_at: None,
        server_pid: 0,
        server_started_at: OffsetDateTime::UNIX_EPOCH,
        schema_version: session::SCHEMA_VERSION_CURRENT,
        ipc_protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
        attached_clients: Vec::new(),
    }
}

#[test]
fn orphan_pid_zero_removes_files_without_signal() {
    // pid==0はkill(2)で自プロセスグループ全体に配送される特殊値で、
    // signalを送るとテストプロセス側が巻き添えで死ぬ。
    // このテストが最後まで走りきりOk(())で戻る事実自体が「signalが送られていない」証拠になる
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let meta = orphan_meta();

    let ctl = paths::ctl_path(base, &meta.id);
    let sock = paths::socket_path(base, &meta.id);
    let meta_path = paths::meta_path(base, &meta.id);
    std::fs::File::create(&ctl).unwrap();
    std::fs::File::create(&sock).unwrap();
    std::fs::File::create(&meta_path).unwrap();

    kill_one_session(base, &meta).unwrap();

    assert!(!ctl.exists());
    assert!(!sock.exists());
    assert!(!meta_path.exists());
}

#[test]
fn orphan_pid_zero_succeeds_when_files_missing() {
    let dir = tempfile::tempdir().unwrap();
    let meta = orphan_meta();
    kill_one_session(dir.path(), &meta).unwrap();
}
