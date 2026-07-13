use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork};
use uuid::Uuid;

use crate::cli::NewArgs;
use crate::server::{self, ServerRunArgs};
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
    let socket_path = paths::socket_path(&base_dir, &session_id);

    // fork前にArgsを組み立てておく(子プロセスで新規allocationを最小にするため)
    let server_args = ServerRunArgs {
        id: session_id,
        name: name.clone(),
        cwd,
        shell,
    };

    // SAFETY: forkの子はasync-signal-safeな関数しか呼べないのが原則。skeeperでは
    // fork前にthreadを起動していないため、子側でmalloc/loggerなどの再入問題が起きない前提でこの制約を緩めている
    // (portable-pty / clap / anyhow等は従来のspawn+exec方式でも同じ環境で動いていた)
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            wait_for_server_ready(&socket_path, child, SERVER_READY_TIMEOUT)?;

            if args.detached {
                println!("Session created: {name}");
                return Ok(());
            }

            client::attach(&socket_path)
        }
        Ok(ForkResult::Child) => {
            // 端末に何も漏らさない・親の端末に依存しない状態にするためstdio3本を/dev/nullに向ける
            redirect_stdio_to_devnull();

            let code = match server::run(server_args) {
                Ok(()) => 0,
                Err(_) => 1,
            };
            // Rustのdestructorをスキップして即抜ける。親側で管理するリソースを子で走らせない意図
            std::process::exit(code);
        }
        Err(e) => Err(anyhow::anyhow!("fork failed: {e}")),
    }
}

fn resolve_shell(arg: Option<&str>, env: Option<&str>) -> PathBuf {
    // 空文字は「未指定」扱いにしたいので、or()の前にfilterを噛ませる
    arg.filter(|s| !s.is_empty())
        .or_else(|| env.filter(|s| !s.is_empty()))
        .map_or_else(|| PathBuf::from("/bin/sh"), PathBuf::from)
}

/// stdin/stdout/stderrを/dev/nullに向け直す。子プロセスから親端末への副作用を断つ。
/// 失敗しても子プロセスは続行する(daemonとして立ち上がる方が優先)
fn redirect_stdio_to_devnull() {
    let Ok(devnull) = File::options().read(true).write(true).open("/dev/null") else {
        return;
    };
    let _ = nix::unistd::dup2_stdin(&devnull);
    let _ = nix::unistd::dup2_stdout(&devnull);
    let _ = nix::unistd::dup2_stderr(&devnull);
}

/// サーバがsocketをbindするまで待つ。以下のいずれかで抜ける:
///   ・socket_pathが現れた → Ok
///   ・childが早期に終了していた → 「起動直後に終了」エラー(3秒待たない)
///   ・タイムアウト → childをSIGKILL+reapしてからエラー(遅延したサーバがゴミを作らないよう掃除)
fn wait_for_server_ready(socket_path: &Path, child: Pid, timeout: Duration) -> anyhow::Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if socket_path.exists() {
            return Ok(());
        }
        match waitpid(child, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => {}
            Ok(_) => anyhow::bail!("Server process exited immediately after startup"),
            Err(e) => anyhow::bail!("Failed to wait for server process: {e}"),
        }
        std::thread::sleep(SERVER_POLL_INTERVAL);
    }
    // タイムアウト: childを回収する。std::process::Child::dropとは違い、fork childは
    // 明示的にkill+reapしないと孤児化する
    let _ = kill(child, Signal::SIGKILL);
    let _ = waitpid(child, None);
    anyhow::bail!("Server startup timed out ({}s)", timeout.as_secs())
}

#[cfg(test)]
#[path = "new_tests.rs"]
mod tests;
