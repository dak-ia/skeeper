use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::session::SessionId;

const APP_NAME: &str = "skeeper";

pub fn runtime_dir() -> Result<PathBuf> {
    let xdg = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    runtime_dir_from(xdg.as_deref(), home.as_deref())
}

fn runtime_dir_from(xdg: Option<&Path>, home: Option<&Path>) -> Result<PathBuf> {
    // 空文字列はXDG Base Dir Specの慣例で「未設定」と同一視する
    if let Some(x) = xdg.filter(|p| !p.as_os_str().is_empty()) {
        return Ok(x.join(APP_NAME));
    }
    if let Some(h) = home.filter(|p| !p.as_os_str().is_empty()) {
        return Ok(h.join(".skeeper").join("run"));
    }
    Err(anyhow!(
        "Failed to determine session directory. Set XDG_RUNTIME_DIR or HOME"
    ))
}

#[must_use]
pub fn meta_path(dir: &Path, id: &SessionId) -> PathBuf {
    dir.join(format!("{id}.json"))
}

#[must_use]
pub fn socket_path(dir: &Path, id: &SessionId) -> PathBuf {
    dir.join(format!("{id}.sock"))
}

/// 制御用ソケットのパス。attach用sockとは別で、detach指示などをやり取りする
#[must_use]
pub fn ctl_path(dir: &Path, id: &SessionId) -> PathBuf {
    dir.join(format!("{id}.ctl"))
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
