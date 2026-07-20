mod common;

use std::fs;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tempfile::TempDir;

use skeeper::ipc::{self, ClientMsg, ServerMsg};
use skeeper::session;

use common::{
    CLEANUP_TIMEOUT, READ_TIMEOUT, READY_TIMEOUT, handshake, pid_alive, spawn_server,
    spawn_server_in,
};

fn skeeper_bin() -> &'static str {
    env!("CARGO_BIN_EXE_skeeper")
}

#[test]
fn kill_by_name_terminates_server_and_cleans_files() -> Result<()> {
    let server = spawn_server("test-kill", "/bin/cat")?;
    let sock = server.socket_path();
    let ctl = server.ctl_path();
    let meta = server.meta_path();

    // 事前確認: サーバ起動時のファイルがすべて揃っている
    assert!(sock.exists() && ctl.exists() && meta.exists());

    // 別プロセスで`skeeper kill test-kill`を実行する。
    // サーバと同じXDG_RUNTIME_DIRを渡さないと別ディレクトリのsessionsを見に行く
    let output = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", server.tmp.path())
        .args(["kill", "test-kill"])
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "kill command should exit successfully. stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // killはSIGTERM後に最大3秒待つので、この時点で全ファイルが掃除されているはず
    assert!(!sock.exists(), "socket should be removed after kill");
    assert!(!ctl.exists(), "ctl socket should be removed after kill");
    assert!(!meta.exists(), "meta should be removed after kill");
    Ok(())
}

#[test]
fn kill_all_terminates_multiple_sessions() -> Result<()> {
    // 2セッションが同じruntime_dir配下に並ぶ状態を作る(kill --allは共有dirを見に行くため)
    let tmp = TempDir::new()?;
    let s1 = spawn_server_in(tmp.path(), "kill-all-1", "/bin/cat")?;
    let s2 = spawn_server_in(tmp.path(), "kill-all-2", "/bin/cat")?;

    let files_before = [
        s1.socket_path(),
        s1.ctl_path(),
        s1.meta_path(),
        s2.socket_path(),
        s2.ctl_path(),
        s2.meta_path(),
    ];
    for p in &files_before {
        assert!(p.exists(), "{} should exist before kill", p.display());
    }

    let mut child = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", tmp.path())
        .args(["kill", "--all"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // "Kill 2 sessions: ...? [y/N] "の確認プロンプトに"y"を返す
    child
        .stdin
        .as_mut()
        .expect("stdin was piped")
        .write_all(b"y\n")?;
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "kill --all should exit successfully. stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    for p in &files_before {
        assert!(
            !p.exists(),
            "{} should be removed after kill --all",
            p.display()
        );
    }

    // 両サーバプロセスがESRCHで消えるまでpoll
    let start = Instant::now();
    while start.elapsed() < CLEANUP_TIMEOUT {
        if !pid_alive(s1.pid()) && !pid_alive(s2.pid()) {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    bail!(
        "server processes still alive after kill --all: s1={} s2={}",
        pid_alive(s1.pid()),
        pid_alive(s2.pid()),
    )
}

#[test]
fn kill_all_with_yes_flag_bypasses_confirmation_on_null_stdin() -> Result<()> {
    // --yes/-y は tty 有無に関係なく確認を省く挙動を検証。
    // 従来は stdin が EOF (Stdio::null) だと confirm() が false を返して "Aborted" 経路になっていた
    let tmp = TempDir::new()?;
    let s1 = spawn_server_in(tmp.path(), "kill-yes-1", "/bin/cat")?;
    let s2 = spawn_server_in(tmp.path(), "kill-yes-2", "/bin/cat")?;

    let output = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", tmp.path())
        .args(["kill", "--all", "--yes"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    assert!(
        output.status.success(),
        "kill --all --yes should succeed even with null stdin. stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("Aborted"),
        "should not print 'Aborted' with --yes"
    );

    // 両サーバが実際に終わっている事を確認
    let start = Instant::now();
    while start.elapsed() < CLEANUP_TIMEOUT {
        if !pid_alive(s1.pid()) && !pid_alive(s2.pid()) {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    bail!("server processes still alive after kill --all --yes");
}

#[test]
fn list_prints_session_names() -> Result<()> {
    let tmp = TempDir::new()?;
    // ハンドルはlist実行中も生かしておく必要があるので`_s1`/`_s2`で束縛(即drop回避)
    let _s1 = spawn_server_in(tmp.path(), "list-one", "/bin/cat")?;
    let _s2 = spawn_server_in(tmp.path(), "list-two", "/bin/cat")?;

    let output = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", tmp.path())
        .arg("list")
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "list should exit successfully. stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("list-one"),
        "stdout should contain 'list-one'\n---stdout---\n{stdout}"
    );
    assert!(
        stdout.contains("list-two"),
        "stdout should contain 'list-two'\n---stdout---\n{stdout}"
    );
    Ok(())
}

#[test]
fn rename_via_cli_updates_meta() -> Result<()> {
    let server = spawn_server("rename-before", "/bin/cat")?;

    // SKEEPER_SESSION_IDを立てて「セッション内で実行された」状態を再現する
    let output = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", server.tmp.path())
        .env("SKEEPER_SESSION_ID", server.id.to_string())
        .args(["rename", "rename-after"])
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "rename should exit successfully. stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // rename CLIは制御ソケットへの投げっぱなし。metaのatomic write完了までpollする
    let start = Instant::now();
    while start.elapsed() < CLEANUP_TIMEOUT {
        if let Ok(meta) = session::read_meta(&server.meta_path())
            && meta.name == "rename-after"
        {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    bail!("meta.name did not update to 'rename-after' within timeout")
}

#[test]
fn prune_removes_orphan_meta() -> Result<()> {
    let server = spawn_server("prune-target", "/bin/cat")?;

    // SIGKILLはSessionFileGuardのdropを通らないので、meta/socket/ctlが残り
    // 「サーバ死+ファイル残」= pruneが掃除すべきorphanの状態を作れる
    signal::kill(server.server_pid, Signal::SIGKILL)?;

    let start = Instant::now();
    while start.elapsed() < CLEANUP_TIMEOUT {
        if !pid_alive(server.server_pid) {
            break;
        }
        sleep(Duration::from_millis(50));
    }
    assert!(
        !pid_alive(server.server_pid),
        "server should be dead after SIGKILL"
    );
    assert!(
        server.meta_path().exists(),
        "meta should still be on disk before prune"
    );

    let output = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", server.tmp.path())
        .arg("prune")
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "prune should exit successfully. stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    assert!(!server.socket_path().exists(), "socket should be pruned");
    assert!(!server.ctl_path().exists(), "ctl should be pruned");
    assert!(!server.meta_path().exists(), "meta should be pruned");
    Ok(())
}

#[test]
fn detach_via_cli_triggers_detach_ack() -> Result<()> {
    let server = spawn_server("detach-cli", "/bin/cat")?;

    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut stream)?;

    // detach.rsは attached_client_pids.is_empty() で先にbailするため、meta更新完了まで待つ
    let start = Instant::now();
    loop {
        if let Ok(m) = session::read_meta(&server.meta_path())
            && !m.attached_client_pids.is_empty()
        {
            break;
        }
        if start.elapsed() >= CLEANUP_TIMEOUT {
            bail!("attached_client_pids did not appear on meta");
        }
        sleep(Duration::from_millis(50));
    }

    // 制御ソケット経由のRequestDetachは「直近stdin送信元client」をdetachする実装なので、
    // detachが確実に効くようにまず何か送ってLAST_STDIN_CLIENTをこのclientに固定する
    ipc::write_client_msg(&mut stream, &ClientMsg::Stdin(b"warmup\n".to_vec()))?;
    // /bin/catのechoを消費して以降のreadでDetachAckを取りこぼさないようにする
    for _ in 0..30 {
        match ipc::read_server_msg(&mut stream) {
            Ok(ServerMsg::Stdout(bytes)) if bytes.windows(6).any(|w| w == b"warmup") => break,
            Ok(_) => (),
            Err(_) => break,
        }
    }

    // 別プロセスで`skeeper detach`を実行(SKEEPER_SESSION_IDでセッション内実行を再現)
    let output = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", server.tmp.path())
        .env("SKEEPER_SESSION_ID", server.id.to_string())
        .arg("detach")
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "detach should exit successfully. stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // /bin/catはStdoutを自発的に出さないので最初のサーバメッセージがDetachAckのはず。
    // 念のためStdoutが混ざる可能性はスキップして数回試す
    let mut got_ack = false;
    for _ in 0..30 {
        match ipc::read_server_msg(&mut stream) {
            Ok(ServerMsg::DetachAck) => {
                got_ack = true;
                break;
            }
            Ok(_) => (),
            Err(_) => break,
        }
    }
    assert!(
        got_ack,
        "attached client should receive DetachAck after `skeeper detach`"
    );
    Ok(())
}

#[test]
fn new_with_name_creates_session() -> Result<()> {
    let tmp = TempDir::new()?;
    let runtime_dir = tmp.path().to_path_buf();
    let base_dir = runtime_dir.join("skeeper");

    let status = Command::new(skeeper_bin())
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["new", "-d", "--shell", "/bin/cat", "new-named"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    assert!(status.success(), "`skeeper new -d NAME` should succeed");

    // -d完了時点でmetaが書かれているはずだが、fs反映の遅延を保険で許容してpollする
    let start = Instant::now();
    let meta = loop {
        let json_count = fs::read_dir(&base_dir).ok().map_or(0, |it| {
            it.flatten()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
                .count()
        });
        let metas = session::list_all_meta(&base_dir).unwrap_or_default();
        if json_count == 1
            && let Some(m) = metas.into_iter().find(|m| m.name == "new-named")
        {
            break m;
        }
        if start.elapsed() >= READY_TIMEOUT {
            let names: Vec<String> = session::list_all_meta(&base_dir)
                .unwrap_or_default()
                .into_iter()
                .map(|m| m.name)
                .collect();
            bail!(
                "expected exactly 1 json meta with name 'new-named'. json_count={json_count} names={names:?}"
            );
        }
        sleep(Duration::from_millis(50));
    };
    assert_eq!(meta.name, "new-named");

    // 後始末: daemon pidをSIGTERM→自然終了しなければSIGKILL(ServerGuard::dropと同じ手順)
    #[allow(clippy::cast_possible_wrap)] // pidは常にpositive i32範囲
    let pid = Pid::from_raw(meta.server_pid as i32);
    let _ = signal::kill(pid, Signal::SIGTERM);
    for _ in 0..30 {
        if !pid_alive(pid) {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    let _ = signal::kill(pid, Signal::SIGKILL);
    Ok(())
}
