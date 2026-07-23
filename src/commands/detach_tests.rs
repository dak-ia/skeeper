use super::*;

use time::OffsetDateTime;
use uuid::Uuid;

fn sample_meta(id: Uuid, attached_pids: Vec<u32>) -> session::SessionMeta {
    let attached_clients = attached_pids
        .into_iter()
        .map(|pid| session::ClientInfo {
            pid,
            tty: None,
            ssh_connection: None,
            attached_at: OffsetDateTime::UNIX_EPOCH,
        })
        .collect();
    session::SessionMeta {
        id,
        name: "test".to_string(),
        cwd: std::path::PathBuf::from("/"),
        shell: std::path::PathBuf::from("/bin/sh"),
        created_at: OffsetDateTime::UNIX_EPOCH,
        last_attached_at: None,
        server_pid: 0,
        server_started_at: OffsetDateTime::UNIX_EPOCH,
        schema_version: session::SCHEMA_VERSION_CURRENT,
        ipc_protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
        attached_clients,
    }
}

#[test]
fn run_errors_when_not_in_session() {
    let _guard = crate::test_helpers::env_lock();
    // XDG/HOMEもtempに寄せて、runtime_dirがテスト外の実状態を参照しないようにする
    let dir = tempfile::tempdir().unwrap();
    unsafe {
        std::env::remove_var("SKEEPER_SESSION_ID");
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }

    assert!(run().is_err());
}

#[test]
fn run_errors_when_no_client_is_attached() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    // runtime_dirはXDG_RUNTIME_DIR/skeeperを返すので、そのサブディレクトリに書く
    let base = dir.path().join("skeeper");
    std::fs::create_dir_all(&base).unwrap();

    let id = Uuid::from_u128(0xdead_beef);
    session::write_meta_atomic(&paths::meta_path(&base, &id), &sample_meta(id, Vec::new()))
        .unwrap();

    unsafe {
        std::env::set_var("SKEEPER_SESSION_ID", id.to_string());
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }

    assert!(run().is_err());
}
