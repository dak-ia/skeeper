use super::*;

use time::OffsetDateTime;

fn sample_meta(id: Uuid) -> session::SessionMeta {
    session::SessionMeta {
        id,
        name: "test".to_string(),
        cwd: std::path::PathBuf::from("/"),
        shell: std::path::PathBuf::from("/bin/sh"),
        created_at: OffsetDateTime::UNIX_EPOCH,
        last_attached_at: None,
        server_pid: 0,
        server_started_at: OffsetDateTime::UNIX_EPOCH,
        attached_client_pids: Vec::new(),
    }
}

#[test]
fn run_prints_help_when_env_unset_and_dir_empty() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    // HOMEもtempにしておく: XDG未設定にした将来変更に備えたフォールバック側の巻き添え防止
    unsafe {
        std::env::remove_var("SKEEPER_SESSION_ID");
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }
    run().unwrap();
}

#[test]
fn run_prints_status_when_env_set_and_meta_exists() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    // runtime_dirはXDG_RUNTIME_DIR/skeeperを返すのでそのサブディレクトリに書き込む
    let base = dir.path().join("skeeper");
    std::fs::create_dir_all(&base).unwrap();
    let id = Uuid::from_u128(0x1234_5678);
    session::write_meta_atomic(&paths::meta_path(&base, &id), &sample_meta(id)).unwrap();
    unsafe {
        std::env::set_var("SKEEPER_SESSION_ID", id.to_string());
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }
    run().unwrap();
}
