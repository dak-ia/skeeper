use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use nix::errno::Errno;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tempfile::TempDir;
use uuid::Uuid;

use skeeper::ipc::{self, ClientMsg, ControlMsg, ServerMsg};
use skeeper::paths;
use skeeper::session;

const READY_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(3);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

/// テスト終了時にサーバプロセスを確実に回収するguard。
/// 通常はSIGTERMでcleanup経路を通す。反応しない場合の保険でSIGKILL。
/// forkで起動したdaemonは`skeeper new -d`の実行プロセスの子孫ではないので、
/// `Child`ではなくpidで管理する
struct ServerGuard {
    server_pid: Pid,
    // TempDirは所有権を保持してテスト終了までpathを生かす
    #[allow(dead_code)]
    tmp: TempDir,
    base_dir: PathBuf,
    id: Uuid,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = signal::kill(self.server_pid, Signal::SIGTERM);
        // 最大1.5秒待って自然終了しなければSIGKILL
        for _ in 0..30 {
            if !pid_alive(self.server_pid) {
                return;
            }
            sleep(Duration::from_millis(50));
        }
        let _ = signal::kill(self.server_pid, Signal::SIGKILL);
        // fork子孫ではないためreapできない。pid消滅を短時間待つ
        for _ in 0..20 {
            if !pid_alive(self.server_pid) {
                return;
            }
            sleep(Duration::from_millis(50));
        }
    }
}

impl ServerGuard {
    fn socket_path(&self) -> PathBuf {
        paths::socket_path(&self.base_dir, &self.id)
    }
    fn ctl_path(&self) -> PathBuf {
        paths::ctl_path(&self.base_dir, &self.id)
    }
    fn meta_path(&self) -> PathBuf {
        paths::meta_path(&self.base_dir, &self.id)
    }
}

/// pidが生きているかを`kill(pid, 0)`で確認する。fork子孫でないpidはwaitpidで回収できないので、
/// pid消滅をpollingで待つときはこの関数を使う
fn pid_alive(pid: Pid) -> bool {
    // EPERM等は「存在するが権限がない」= 生きているとみなす。テスト実行者と同じUIDなので通常は起きない
    !matches!(signal::kill(pid, None), Err(Errno::ESRCH))
}

/// テスト用のサーバをspawn。TempDirをXDG_RUNTIME_DIRとして渡し、他テストと隔離する。
/// `skeeper new -d`を呼ぶとforkで裏にdaemonが立ち上がる。実行プロセス自体は
/// "Session created: {name}"を出して即終了するので、waitで回収してからmetaを読む
fn spawn_server(name: &str, shell: &str) -> Result<ServerGuard> {
    let tmp = TempDir::new()?;
    let runtime_dir = tmp.path().to_path_buf();

    let bin = env!("CARGO_BIN_EXE_skeeper");
    let status = Command::new(bin)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["new", "-d", "--shell", shell, name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("`skeeper new -d` exited with {status}");
    }

    let base_dir = runtime_dir.join("skeeper");
    // 指定nameのメタが現れるまで待つ(-d完了時点で書き込み済のはずだが、fs反映の遅延を保険で許容)
    let start = Instant::now();
    let (id, server_pid) = loop {
        if let Ok(metas) = session::list_all_meta(&base_dir) {
            if let Some(m) = metas.into_iter().find(|m| m.name == name) {
                #[allow(clippy::cast_possible_wrap)] // pidは常にpositive i32範囲
                break (m.id, Pid::from_raw(m.server_pid as i32));
            }
        }
        if start.elapsed() >= READY_TIMEOUT {
            bail!("meta for '{name}' did not appear within {READY_TIMEOUT:?}");
        }
        sleep(Duration::from_millis(50));
    };

    let guard = ServerGuard {
        server_pid,
        tmp,
        base_dir,
        id,
    };

    // socket作成完了もwait(-d完了時点で完了しているはずの二重確認)
    let sock = guard.socket_path();
    let start = Instant::now();
    while start.elapsed() < READY_TIMEOUT {
        if sock.exists() {
            return Ok(guard);
        }
        sleep(Duration::from_millis(50));
    }
    bail!("server did not create socket within {READY_TIMEOUT:?}")
}

fn handshake(stream: &mut UnixStream) -> Result<()> {
    ipc::write_client_msg(
        stream,
        &ClientMsg::Hello {
            client_pid: std::process::id(),
            cols: 80,
            rows: 24,
        },
    )?;
    let resp = ipc::read_server_msg(stream)?;
    match resp {
        ServerMsg::HelloOk { .. } => Ok(()),
        other => bail!("expected HelloOk, got {other:?}"),
    }
}

#[test]
fn server_creates_expected_files_and_reads_meta() -> Result<()> {
    let server = spawn_server("test-startup", "/bin/cat")?;

    assert!(server.socket_path().exists(), ".sock should exist");
    assert!(server.ctl_path().exists(), ".ctl should exist");
    assert!(server.meta_path().exists(), ".json should exist");

    let meta = session::read_meta(&server.meta_path())?;
    assert_eq!(meta.name, "test-startup");
    assert_eq!(meta.attached_client_pid, None);
    assert_eq!(meta.last_attached_at, None);

    Ok(())
}

#[test]
fn sigterm_triggers_cleanup() -> Result<()> {
    let server = spawn_server("test-sigterm", "/bin/cat")?;
    let sock = server.socket_path();
    let ctl = server.ctl_path();
    let meta = server.meta_path();

    signal::kill(server.server_pid, Signal::SIGTERM)?;

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
fn client_handshake_succeeds() -> Result<()> {
    let server = spawn_server("test-handshake", "/bin/cat")?;
    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut stream)?;
    Ok(())
}

#[test]
fn meta_shows_attached_after_handshake() -> Result<()> {
    let server = spawn_server("test-meta-attach", "/bin/cat")?;

    // 初期状態: 未接続
    let before = session::read_meta(&server.meta_path())?;
    assert_eq!(before.attached_client_pid, None);
    assert_eq!(before.last_attached_at, None);

    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut stream)?;

    // meta atomic write完了まで少し待つ
    sleep(Duration::from_millis(200));

    // handshake後: attached_client_pidとlast_attached_atが埋まっている
    let after = session::read_meta(&server.meta_path())?;
    assert!(
        after.attached_client_pid.is_some(),
        "attached_client_pid should be Some after handshake"
    );
    assert!(
        after.last_attached_at.is_some(),
        "last_attached_at should be Some after handshake"
    );
    assert_eq!(
        after.attached_client_pid,
        Some(std::process::id()),
        "attached_client_pid should match our process id"
    );

    Ok(())
}

#[test]
fn control_detach_sends_detach_ack() -> Result<()> {
    let server = spawn_server("test-ctldetach", "/bin/cat")?;

    let mut stream = UnixStream::connect(server.socket_path())?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut stream)?;

    // handle_client内での登録完了待ち
    sleep(Duration::from_millis(200));

    // 制御ソケット経由でRequestDetach
    let mut ctl = UnixStream::connect(server.ctl_path())?;
    ipc::write_control_msg(&mut ctl, &ControlMsg::RequestDetach)?;
    drop(ctl);

    // DetachAckが届くはず (/bin/catは自発的にStdoutを出さないので、最初に来るサーバメッセージはDetachAck)
    // 念のため数回ループでStdoutをskip
    let mut got_ack = false;
    for _ in 0..20 {
        match ipc::read_server_msg(&mut stream) {
            Ok(ServerMsg::DetachAck) => {
                got_ack = true;
                break;
            }
            Ok(_) => (),
            Err(_) => break,
        }
    }
    assert!(got_ack, "expected DetachAck within reasonable time");

    Ok(())
}

#[test]
fn control_rename_updates_meta() -> Result<()> {
    let server = spawn_server("test-rename-before", "/bin/cat")?;

    let before = session::read_meta(&server.meta_path())?;
    assert_eq!(before.name, "test-rename-before");

    let mut ctl = UnixStream::connect(server.ctl_path())?;
    ipc::write_control_msg(
        &mut ctl,
        &ControlMsg::RequestRename {
            new_name: "test-rename-after".to_string(),
        },
    )?;
    drop(ctl);

    // metaがatomic write完了するまで待つ
    let start = Instant::now();
    while start.elapsed() < CLEANUP_TIMEOUT {
        if let Ok(meta) = session::read_meta(&server.meta_path()) {
            if meta.name == "test-rename-after" {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(50));
    }
    bail!("meta.name did not update to 'test-rename-after' within timeout")
}

#[test]
fn scrollback_is_replayed_on_new_client() -> Result<()> {
    // /bin/catはstdinをそのままstdoutに返すので、まず1個目のクライアントから
    // 何か入力→cat経由でstdoutに出る→サーバのscrollbackに溜まる
    // 2個目のattachでその内容がStdoutとして再生されるはず
    let server = spawn_server("test-scrollback", "/bin/cat")?;

    let mut c1 = UnixStream::connect(server.socket_path())?;
    c1.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut c1)?;
    sleep(Duration::from_millis(100));

    // stdinに投げるとcatがstdoutに折り返す
    ipc::write_client_msg(&mut c1, &ClientMsg::Stdin(b"hello-scrollback\n".to_vec()))?;

    // c1でエコーが返るのを待つ
    let mut c1_saw_echo = false;
    for _ in 0..30 {
        if let Ok(ServerMsg::Stdout(bytes)) = ipc::read_server_msg(&mut c1) {
            if bytes.windows(16).any(|w| w == b"hello-scrollback") {
                c1_saw_echo = true;
                break;
            }
        }
    }
    assert!(
        c1_saw_echo,
        "first client should have echoed hello-scrollback"
    );

    // c1をdetach
    let mut ctl = UnixStream::connect(server.ctl_path())?;
    ipc::write_control_msg(&mut ctl, &ControlMsg::RequestDetach)?;
    drop(ctl);
    // DetachAckまで消費
    for _ in 0..20 {
        if let Ok(ServerMsg::DetachAck) = ipc::read_server_msg(&mut c1) {
            break;
        }
    }
    drop(c1);
    sleep(Duration::from_millis(200));

    // c2で再attach → HelloOk直後にscrollbackが再生されるはず
    let mut c2 = UnixStream::connect(server.socket_path())?;
    c2.set_read_timeout(Some(READ_TIMEOUT))?;
    handshake(&mut c2)?;

    // c2の最初のStdoutメッセージがscrollback (hello-scrollbackを含む)
    let mut c2_saw_replay = false;
    for _ in 0..10 {
        if let Ok(ServerMsg::Stdout(bytes)) = ipc::read_server_msg(&mut c2) {
            if bytes.windows(16).any(|w| w == b"hello-scrollback") {
                c2_saw_replay = true;
                break;
            }
        }
    }
    assert!(c2_saw_replay, "second client should see scrollback replay");
    Ok(())
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
    let bin = env!("CARGO_BIN_EXE_skeeper");
    let output = Command::new(bin)
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
