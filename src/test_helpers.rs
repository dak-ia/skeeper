use std::sync::{Mutex, MutexGuard};

/// process-globalな env var (XDG_RUNTIME_DIR / SKEEPER_SESSION_ID など) を触るテスト間で
/// 並列実行の衝突を避けるためのlock。テスト先頭で `let _guard = env_lock();` する。
///
/// テストがpanicしてもpoisonされたlockを次のテストが使えるようにinto_inner()する
pub fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
