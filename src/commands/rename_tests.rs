use super::*;

use time::OffsetDateTime;
use uuid::Uuid;

use crate::cli::RenameArgs;

fn sample_meta(id: Uuid, name: &str) -> session::SessionMeta {
    session::SessionMeta {
        id,
        name: name.to_string(),
        cwd: std::path::PathBuf::from("/"),
        shell: std::path::PathBuf::from("/bin/sh"),
        created_at: OffsetDateTime::UNIX_EPOCH,
        last_attached_at: None,
        server_pid: 0,
        server_started_at: OffsetDateTime::UNIX_EPOCH,
        attached_client_pid: None,
    }
}

#[test]
fn run_errors_when_no_old_and_not_in_session() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    unsafe {
        std::env::remove_var("SKEEPER_SESSION_ID");
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }

    assert!(
        run(RenameArgs {
            new_name: "whatever".to_string(),
            old: None,
        })
        .is_err()
    );
}

#[test]
fn run_errors_when_named_session_not_found() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    // runtime_dirはXDG_RUNTIME_DIR/skeeperを返すので、空でもそのサブディレクトリを用意する
    let base = dir.path().join("skeeper");
    std::fs::create_dir_all(&base).unwrap();
    unsafe {
        std::env::remove_var("SKEEPER_SESSION_ID");
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }

    assert!(
        run(RenameArgs {
            new_name: "whatever".to_string(),
            old: Some("does-not-exist".to_string()),
        })
        .is_err()
    );
}

#[test]
fn run_errors_when_new_name_collides_with_other_session() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("skeeper");
    std::fs::create_dir_all(&base).unwrap();

    // 衝突検出はctl socketに触れる前に行われるので、meta.jsonが2つあれば足りる
    let id_a = Uuid::from_u128(0xa);
    let id_b = Uuid::from_u128(0xb);
    session::write_meta_atomic(&paths::meta_path(&base, &id_a), &sample_meta(id_a, "alpha"))
        .unwrap();
    session::write_meta_atomic(&paths::meta_path(&base, &id_b), &sample_meta(id_b, "beta"))
        .unwrap();

    unsafe {
        std::env::remove_var("SKEEPER_SESSION_ID");
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }

    assert!(
        run(RenameArgs {
            new_name: "beta".to_string(),
            old: Some("alpha".to_string()),
        })
        .is_err()
    );
}
