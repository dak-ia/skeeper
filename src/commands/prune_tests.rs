use super::*;

use time::OffsetDateTime;
use uuid::Uuid;

#[test]
fn run_returns_ok_when_no_sessions() {
    let _guard = crate::test_helpers::env_lock();
    // XDG/HOMEをtempに寄せて、runtime_dirがテスト外の実状態を参照しないようにする
    let dir = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }
    // runtime_dir配下はまだ作っておらず、list_all_metaはNotFoundを空Vecとして返す
    run().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn run_prunes_meta_with_server_pid_zero() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    // runtime_dirはXDG_RUNTIME_DIR/skeeperを返すので、そのサブディレクトリに書く
    let base = dir.path().join("skeeper");
    std::fs::create_dir_all(&base).unwrap();

    let id = Uuid::from_u128(0xdead_beef);
    let meta = session::SessionMeta {
        id,
        name: "orphan".to_string(),
        cwd: std::path::PathBuf::from("/"),
        shell: std::path::PathBuf::from("/bin/sh"),
        created_at: OffsetDateTime::UNIX_EPOCH,
        last_attached_at: None,
        server_pid: 0,
        server_started_at: OffsetDateTime::UNIX_EPOCH,
        attached_client_pid: None,
    };
    let meta_path = paths::meta_path(&base, &id);
    let sock = paths::socket_path(&base, &id);
    let ctl = paths::ctl_path(&base, &id);
    session::write_meta_atomic(&meta_path, &meta).unwrap();
    std::fs::File::create(&sock).unwrap();
    std::fs::File::create(&ctl).unwrap();

    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }

    run().unwrap();

    assert!(!meta_path.exists());
    assert!(!sock.exists());
    assert!(!ctl.exists());
}
