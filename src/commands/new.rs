use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::Context;
use uuid::Uuid;

use crate::cli::NewArgs;
use crate::{client, name_gen, paths, session};

use super::current_session_id;

const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn run(args: NewArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    std::fs::create_dir_all(&base_dir)?;

    if current_session_id(&base_dir).is_some() {
        anyhow::bail!("Already inside a session. Run `skeeper d` to detach first");
    }

    let shell = resolve_shell(
        args.shell.as_deref(),
        std::env::var("SHELL").ok().as_deref(),
    );
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let name = args
        .name
        .unwrap_or_else(|| name_gen::random_name(&mut rand::rng()));

    // 名前衝突チェック(list_all_metaはパース失敗を黙って飛ばすのでunwrap_or_defaultで十分)
    let existing = session::list_all_meta(&base_dir).unwrap_or_default();
    if existing.iter().any(|m| m.name == name) {
        anyhow::bail!(
            "Session name '{name}' is already in use. Run `skeeper attach {name}` to attach"
        );
    }

    let session_id = Uuid::new_v4();
    let self_exe = std::env::current_exe().context("Failed to get the current executable path")?;

    let mut child = std::process::Command::new(&self_exe)
        .arg("__server-run")
        .arg("--id")
        .arg(session_id.to_string())
        .arg("--name")
        .arg(&name)
        .arg("--cwd")
        .arg(&cwd)
        .arg("--shell")
        .arg(&shell)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to start the server process")?;

    let socket_path = paths::socket_path(&base_dir, &session_id);
    wait_for_server_ready(&socket_path, &mut child, SERVER_READY_TIMEOUT)?;

    if args.detached {
        println!("Session created: {name}");
        return Ok(());
    }

    client::attach(&socket_path)
}

fn resolve_shell(arg: Option<&str>, env: Option<&str>) -> PathBuf {
    // 空文字は「未指定」扱いにしたいので、or()の前にfilterを噛ませる
    arg.filter(|s| !s.is_empty())
        .or_else(|| env.filter(|s| !s.is_empty()))
        .map_or_else(|| PathBuf::from("/bin/sh"), PathBuf::from)
}

/// サーバがsocketをbindするまで待つ。以下のいずれかで抜ける:
///   ・socket_pathが現れた → Ok
///   ・childが早期に終了していた → 「起動直後に終了」エラー(3秒待たない)
///   ・タイムアウト → childをkill+reapしてからエラー(遅延したサーバがゴミを作らないよう掃除)
fn wait_for_server_ready(
    socket_path: &Path,
    child: &mut std::process::Child,
    timeout: Duration,
) -> anyhow::Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if socket_path.exists() {
            return Ok(());
        }
        if let Ok(Some(_status)) = child.try_wait() {
            anyhow::bail!("Server process exited immediately after startup");
        }
        std::thread::sleep(SERVER_POLL_INTERVAL);
    }
    // タイムアウト: childを回収する。Rustのstd::process::Child::dropはkillしないので
    // 明示的に始末しないと遅延起動して孤児化する
    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!("Server startup timed out ({}s)", timeout.as_secs())
}

#[cfg(test)]
#[path = "new_tests.rs"]
mod tests;
