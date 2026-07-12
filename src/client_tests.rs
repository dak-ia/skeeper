use super::*;

use tempfile::tempdir;

#[test]
fn attach_errors_when_socket_path_does_not_exist() {
    let dir = tempdir().unwrap();
    // 実在しないpath: connectが最初のI/OなのでTerminalGuard::enter()には到達せず、
    // テスト実行環境(TTY無し)でも安全に落ちる
    let missing = dir.path().join("nonexistent.sock");

    let err = attach(&missing).expect_err("connect must fail for missing socket");
    let msg = format!("{err:#}");
    assert!(
        msg.contains(&missing.display().to_string()),
        "error should mention the socket path, got: {msg}"
    );
}
