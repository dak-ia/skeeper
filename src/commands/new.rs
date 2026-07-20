use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::cli::NewArgs;
use crate::server::{self, ServerRunArgs};
use crate::session::SessionMeta;
use crate::{client, name_gen, paths, runtime_lock, session};

use super::current_session_id;

const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn run(args: NewArgs) -> anyhow::Result<()> {
    let NewArgs {
        name: requested_name,
        detached,
        shell: shell_arg,
        cwd: cwd_arg,
    } = args;

    let base_dir = paths::runtime_dir()?;
    paths::ensure_runtime_dir(&base_dir)?;

    if current_session_id(&base_dir).is_some() {
        anyhow::bail!("Already inside a session. Run `skeeper d` to detach first");
    }

    let shell = resolve_shell(shell_arg.as_deref(), std::env::var("SHELL").ok().as_deref());
    let current = std::env::current_dir().context("Failed to get current directory")?;
    let cwd = resolve_cwd(cwd_arg.as_deref(), &current)?;

    let session_id = Uuid::new_v4();
    let meta_path = paths::meta_path(&base_dir, &session_id);
    let socket_path = paths::socket_path(&base_dir, &session_id);

    // 名前選定と予約(server_pid=0のmeta)書き込みをflock内で完結させる。
    // 書き終えたらscopeを抜けてflockを解放し、child(server)が親のflock fdを継承しないようにする
    let name = {
        let _lock = runtime_lock::acquire_runtime_lock(&base_dir)?;
        let taken: HashSet<String> = session::list_all_meta(&base_dir)
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.name)
            .collect();
        let name = match requested_name {
            Some(n) => {
                if taken.contains(&n) {
                    bail!(
                        "Session name '{n}' is already in use. Run `skeeper attach {n}` to attach"
                    );
                }
                n
            }
            None => name_gen::pick_available_name(&mut rand::rng(), &taken).with_context(|| {
                format!(
                    "No available session name ({} name space exhausted)",
                    name_gen::TOTAL_NAMES
                )
            })?,
        };
        // reservation: server_pid=0の状態でmeta書き込みして名前を占有する。
        // server起動時にwrite_meta_atomicで正しいserver_pid/started_atに上書きされる
        let reservation = SessionMeta {
            id: session_id,
            name: name.clone(),
            cwd: cwd.clone(),
            shell: shell.clone(),
            created_at: OffsetDateTime::now_utc(),
            last_attached_at: None,
            server_pid: 0,
            server_started_at: OffsetDateTime::UNIX_EPOCH,
            attached_client_pids: Vec::new(),
        };
        session::write_meta_atomic(&meta_path, &reservation)?;
        name
    };

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
            if let Err(e) = wait_for_server_ready(&socket_path, child, SERVER_READY_TIMEOUT) {
                // startup失敗時は予約metaを掃除(orphanのままlist/pickに影響しないよう)
                let _ = std::fs::remove_file(&meta_path);
                return Err(e);
            }

            if detached {
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
        Err(e) => {
            let _ = std::fs::remove_file(&meta_path);
            Err(anyhow::anyhow!("fork failed: {e}"))
        }
    }
}

fn resolve_shell(arg: Option<&str>, env: Option<&str>) -> PathBuf {
    // 空文字は「未指定」扱いにしたいので、or()の前にfilterを噛ませる
    arg.filter(|s| !s.is_empty())
        .or_else(|| env.filter(|s| !s.is_empty()))
        .map_or_else(|| PathBuf::from("/bin/sh"), PathBuf::from)
}

/// --cwd引数を解決する。指定なしなら現在のcwdを踏襲、指定ありは相対→絶対化+存在/is_dir検証
fn resolve_cwd(arg: Option<&Path>, current: &Path) -> anyhow::Result<PathBuf> {
    let Some(p) = arg else {
        return Ok(current.to_path_buf());
    };
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        current.join(p)
    };
    let meta = std::fs::metadata(&abs)
        .with_context(|| format!("--cwd '{}' is not accessible", abs.display()))?;
    if !meta.is_dir() {
        bail!("--cwd '{}' is not a directory", abs.display());
    }
    Ok(abs)
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
