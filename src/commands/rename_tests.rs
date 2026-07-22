use super::*;

use crate::cli::RenameArgs;

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
