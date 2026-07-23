use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tempfile::TempDir;

use skeeper::{paths, session};

mod common;

use common::{CLEANUP_TIMEOUT, READY_TIMEOUT, spawn_server};

#[test]
fn server_creates_expected_files_and_reads_meta() -> Result<()> {
    let server = spawn_server("test-startup", "/bin/cat")?;

    assert!(server.socket_path().exists(), ".sock should exist");
    assert!(server.ctl_path().exists(), ".ctl should exist");
    assert!(server.meta_path().exists(), ".json should exist");

    let meta = session::read_meta(&server.meta_path())?;
    assert_eq!(meta.name, "test-startup");
    assert!(meta.attached_clients.is_empty());
    assert_eq!(meta.last_attached_at, None);

    Ok(())
}

#[test]
fn sigterm_triggers_cleanup() -> Result<()> {
    let server = spawn_server("test-sigterm", "/bin/cat")?;
    let sock = server.socket_path();
    let ctl = server.ctl_path();
    let meta = server.meta_path();

    signal::kill(server.pid(), Signal::SIGTERM)?;

    let start = Instant::now();
    while start.elapsed() < CLEANUP_TIMEOUT {
        if !sock.exists() && !ctl.exists() && !meta.exists() {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }

    bail!(
        "files not cleaned within {CLEANUP_TIMEOUT:?}: sock={}, ctl={}, meta={}",
        sock.exists(),
        ctl.exists(),
        meta.exists(),
    )
}

#[test]
fn fork_daemon_persists_after_parent_new_exit() -> Result<()> {
    // `skeeper new -d NAME`はforkでdaemonを立てた後、親自身は即exitする。
    // 親が消えてもdaemonが動き続け、socket/metaが残っていることを確認する
    let tmp = TempDir::new()?;
    let runtime_dir = tmp.path().to_path_buf();
    let name = "test-fork-persist";

    let bin = env!("CARGO_BIN_EXE_skeeper");
    let mut child = Command::new(bin)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["new", "-d", "--shell", "/bin/cat", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // 親のpidは`skeeper new`プロセス自身のもの。daemonはforkの子でこれとは別pidになる
    let parent_pid = child.id();
    let status = child.wait()?;
    if !status.success() {
        bail!("`skeeper new -d` parent exited with {status}");
    }

    let base_dir = runtime_dir.join("skeeper");
    let start = Instant::now();
    let meta = loop {
        if let Ok(metas) = session::list_all_meta(&base_dir)
            && let Some(m) = metas.into_iter().find(|m| m.name == name)
        {
            break m;
        }
        if start.elapsed() >= READY_TIMEOUT {
            bail!("meta for '{name}' did not appear within {READY_TIMEOUT:?}");
        }
        sleep(Duration::from_millis(50));
    };

    assert_ne!(
        meta.server_pid, parent_pid,
        "daemon pid should differ from `skeeper new` parent pid"
    );

    #[allow(clippy::cast_possible_wrap)] // pidは常にpositive i32範囲
    let daemon_pid = Pid::from_raw(meta.server_pid as i32);

    // 親exit後もdaemonは生きている。ESRCH以外(EPERM含む)は生存扱いだが、
    // 通常はtest実行者と同一UIDなのでOkが返る
    assert!(
        signal::kill(daemon_pid, None).is_ok(),
        "daemon pid={} should be alive after parent exit",
        meta.server_pid,
    );

    let sock = paths::socket_path(&base_dir, &meta.id);
    let meta_path = paths::meta_path(&base_dir, &meta.id);
    assert!(sock.exists(), "socket should exist after parent exit");
    assert!(meta_path.exists(), "meta should exist after parent exit");

    // 後始末: SIGTERMでcleanup経路を通す。反応しなければSIGKILLで保険をかける。
    // ServerGuard::dropと同じ手順だが、この関数はguardを使っていないので手で書く
    let _ = signal::kill(daemon_pid, Signal::SIGTERM);
    for _ in 0..30 {
        if signal::kill(daemon_pid, None).is_err() {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    let _ = signal::kill(daemon_pid, Signal::SIGKILL);
    for _ in 0..20 {
        if signal::kill(daemon_pid, None).is_err() {
            break;
        }
        sleep(Duration::from_millis(50));
    }
    Ok(())
}
