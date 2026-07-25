use super::*;

use time::OffsetDateTime;
use uuid::Uuid;

use crate::cli::KillArgs;

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

    let outcome = kill_one_session(base, &meta).unwrap();
    assert!(matches!(outcome, KillOutcome::Killed));
    assert!(!ctl.exists());
    assert!(!sock.exists());
    assert!(!meta_path.exists());
}

#[test]
fn orphan_pid_zero_succeeds_when_files_missing() {
    let dir = tempfile::tempdir().unwrap();
    let meta = orphan_meta();
    let outcome = kill_one_session(dir.path(), &meta).unwrap();
    assert!(matches!(outcome, KillOutcome::Killed));
}

#[test]
fn run_errors_with_ls_hint_when_named_session_not_found() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("skeeper");
    std::fs::create_dir_all(&base).unwrap();
    unsafe {
        std::env::remove_var("SKEEPER_SESSION_ID");
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }

    let err = run(KillArgs {
        name: Some("does-not-exist".to_string()),
        all: false,
        yes: false,
    })
    .expect_err("expected error for missing session");
    // 復旧誘導としてsession一覧確認コマンドを案内している
    assert!(
        err.to_string().contains("skeeper ls"),
        "err message should suggest `skeeper ls`, got: {err}"
    );
}
