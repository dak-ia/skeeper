use super::SessionFileGuard;
use tempfile::tempdir;

#[test]
fn session_file_guard_drop_removes_meta_socket_and_ctl() {
    let dir = tempdir().unwrap();
    let meta = dir.path().join("m.json");
    let sock = dir.path().join("s.sock");
    let ctl = dir.path().join("c.ctl");
    std::fs::write(&meta, b"meta").unwrap();
    std::fs::write(&sock, b"sock").unwrap();
    std::fs::write(&ctl, b"ctl").unwrap();

    drop(SessionFileGuard {
        meta_path: &meta,
        socket_path: &sock,
        ctl_socket_path: &ctl,
    });

    assert!(!meta.exists());
    assert!(!sock.exists());
    assert!(!ctl.exists());
}

#[test]
fn session_file_guard_drop_ignores_missing_files_without_panic() {
    // 二重起動/クラッシュ後の起動で対象ファイルが最初から無いことがあるので、
    // すべて存在しなくてもdropはpanicしないことを保証する
    let dir = tempdir().unwrap();
    let meta = dir.path().join("absent_m.json");
    let sock = dir.path().join("absent_s.sock");
    let ctl = dir.path().join("absent_c.ctl");

    drop(SessionFileGuard {
        meta_path: &meta,
        socket_path: &sock,
        ctl_socket_path: &ctl,
    });
}
