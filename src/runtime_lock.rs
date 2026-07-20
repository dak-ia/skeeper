use std::fs::{File, OpenOptions};
use std::path::Path;

use anyhow::Context;
use nix::fcntl::{Flock, FlockArg};

const LOCK_FILE_NAME: &str = ".lock";

/// runtime_dir下の`.lock`ファイルへの排他flockを保持するRAII guard。
/// dropでflockが解除される
pub struct RuntimeLock(#[allow(dead_code)] Flock<File>);

/// runtime_dir下の`.lock`ファイルにflock(LOCK_EX)を取ってguardを返す。
/// 「list_all_meta → 検証 → 書き込み」の間の他プロセス割り込み防止に使う。
/// forkする前は明示drop(または呼出scopeを閉じる)して子プロセスにfd継承させないこと
pub fn acquire_runtime_lock(base_dir: &Path) -> anyhow::Result<RuntimeLock> {
    let lock_path = base_dir.join(LOCK_FILE_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("Failed to open lock file: {}", lock_path.display()))?;
    let guard = Flock::lock(file, FlockArg::LockExclusive)
        .map_err(|(_, e)| anyhow::anyhow!("Failed to acquire runtime lock: {e}"))?;
    Ok(RuntimeLock(guard))
}

#[cfg(test)]
#[path = "runtime_lock_tests.rs"]
mod tests;
