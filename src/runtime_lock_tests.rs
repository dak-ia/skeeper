use super::*;

#[test]
fn acquire_creates_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let _lock = acquire_runtime_lock(dir.path()).unwrap();
    assert!(dir.path().join(".lock").exists());
}

#[test]
fn drop_releases_lock_so_second_acquire_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    {
        let _lock = acquire_runtime_lock(dir.path()).unwrap();
    }
    // scope out → drop → 再取得可能
    let _lock2 = acquire_runtime_lock(dir.path()).unwrap();
}

#[test]
fn explicit_drop_releases_lock() {
    let dir = tempfile::tempdir().unwrap();
    let lock = acquire_runtime_lock(dir.path()).unwrap();
    drop(lock);
    let _lock2 = acquire_runtime_lock(dir.path()).unwrap();
}
