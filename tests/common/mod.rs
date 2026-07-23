// integration testはcrateごとに独立コンパイルされるため、
// このmoduleを参照しないテストからは全項目がdead_codeに見える
#![allow(dead_code)]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use nix::errno::Errno;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tempfile::TempDir;
use uuid::Uuid;

use skeeper::ipc::{self, ClientMsg, ServerMsg};
use skeeper::paths;
use skeeper::session;

pub const READY_TIMEOUT: Duration = Duration::from_secs(5);
pub const READ_TIMEOUT: Duration = Duration::from_secs(3);
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

/// テスト終了時にサーバプロセスを確実に回収するguard。
/// 通常はSIGTERMでcleanup経路を通す。反応しない場合の保険でSIGKILL。
/// forkで起動したdaemonは`skeeper new -d`の実行プロセスの子孫ではないので、
/// `Child`ではなくpidで管理する
pub struct ServerGuard {
    pub server_pid: Pid,
    // TempDirは所有権を保持してテスト終了までpathを生かす
    pub tmp: TempDir,
    pub base_dir: PathBuf,
    pub id: Uuid,
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
    pub fn socket_path(&self) -> PathBuf {
        paths::socket_path(&self.base_dir, &self.id)
    }
    pub fn ctl_path(&self) -> PathBuf {
        paths::ctl_path(&self.base_dir, &self.id)
    }
    pub fn meta_path(&self) -> PathBuf {
        paths::meta_path(&self.base_dir, &self.id)
    }
    pub fn pid(&self) -> Pid {
        self.server_pid
    }
}

/// pidが生きているかを`kill(pid, 0)`で確認する。fork子孫でないpidはwaitpidで回収できないので、
/// pid消滅をpollingで待つときはこの関数を使う
pub fn pid_alive(pid: Pid) -> bool {
    // EPERM等は「存在するが権限がない」= 生きているとみなす。テスト実行者と同じUIDなので通常は起きない
    !matches!(signal::kill(pid, None), Err(Errno::ESRCH))
}

/// 呼び出し側が保持する`XDG_RUNTIME_DIR`(通常は`TempDir`)へ複数セッションを立てるときのハンドル。
/// `ServerGuard`と違いTempDirを所有せず、複数のハンドルで同じruntime_dirを共有できる
pub struct SessionHandle {
    server_pid: Pid,
    pub base_dir: PathBuf,
    pub id: Uuid,
    pub name: String,
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        let _ = signal::kill(self.server_pid, Signal::SIGTERM);
        for _ in 0..30 {
            if !pid_alive(self.server_pid) {
                return;
            }
            sleep(Duration::from_millis(50));
        }
        let _ = signal::kill(self.server_pid, Signal::SIGKILL);
        for _ in 0..20 {
            if !pid_alive(self.server_pid) {
                return;
            }
            sleep(Duration::from_millis(50));
        }
    }
}

impl SessionHandle {
    pub fn socket_path(&self) -> PathBuf {
        paths::socket_path(&self.base_dir, &self.id)
    }
    pub fn ctl_path(&self) -> PathBuf {
        paths::ctl_path(&self.base_dir, &self.id)
    }
    pub fn meta_path(&self) -> PathBuf {
        paths::meta_path(&self.base_dir, &self.id)
    }
    pub fn pid(&self) -> Pid {
        self.server_pid
    }
}

/// 指定した`runtime_dir`を`XDG_RUNTIME_DIR`として`skeeper new -d`を呼び、
/// 立ち上がったdaemonを`SessionHandle`で返す。同じruntime_dirに複数セッションを並べるときに使う
pub fn spawn_server_in(runtime_dir: &Path, name: &str, shell: &str) -> Result<SessionHandle> {
    let bin = env!("CARGO_BIN_EXE_skeeper");
    let status = Command::new(bin)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .args(["new", "-d", "--shell", shell, name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("`skeeper new -d` exited with {status}");
    }

    let base_dir = runtime_dir.join("skeeper");
    let start = Instant::now();
    let handle = loop {
        if let Ok(metas) = session::list_all_meta(&base_dir)
            && let Some(m) = metas.into_iter().find(|m| m.name == name)
        {
            #[allow(clippy::cast_possible_wrap)] // pidは常にpositive i32範囲
            let pid = Pid::from_raw(m.server_pid as i32);
            break SessionHandle {
                server_pid: pid,
                base_dir: base_dir.clone(),
                id: m.id,
                name: name.to_string(),
            };
        }
        if start.elapsed() >= READY_TIMEOUT {
            bail!("meta for '{name}' did not appear within {READY_TIMEOUT:?}");
        }
        sleep(Duration::from_millis(50));
    };

    let sock = handle.socket_path();
    let start = Instant::now();
    while start.elapsed() < READY_TIMEOUT {
        if sock.exists() {
            return Ok(handle);
        }
        sleep(Duration::from_millis(50));
    }
    bail!("server did not create socket within {READY_TIMEOUT:?}")
}

/// テスト用のサーバをspawn。TempDirをXDG_RUNTIME_DIRとして渡し、他テストと隔離する。
/// `skeeper new -d`を呼ぶとforkで裏にdaemonが立ち上がる。実行プロセス自体は
/// "Session created: {name}"を出して即終了するので、waitで回収してからmetaを読む
pub fn spawn_server(name: &str, shell: &str) -> Result<ServerGuard> {
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
        if let Ok(metas) = session::list_all_meta(&base_dir)
            && let Some(m) = metas.into_iter().find(|m| m.name == name)
        {
            #[allow(clippy::cast_possible_wrap)] // pidは常にpositive i32範囲
            break (m.id, Pid::from_raw(m.server_pid as i32));
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

pub fn handshake(stream: &mut UnixStream) -> Result<()> {
    handshake_as(stream, std::process::id())
}

/// テスト内で1プロセスから複数clientを模擬するためのHelloバリアント。
/// server側は client_pid をユニークキーとして扱うので、同じテスト内で複数attach
/// させたいときは互いに異なる値を渡す
pub fn handshake_as(stream: &mut UnixStream, client_pid: u32) -> Result<()> {
    ipc::write_client_msg(
        stream,
        &ClientMsg::Hello {
            client_pid,
            cols: 80,
            rows: 24,
            tty: None,
            ssh_connection: None,
        },
    )?;
    let resp = ipc::read_server_msg(stream)?;
    match resp {
        ServerMsg::HelloOk { .. } => Ok(()),
        other => bail!("expected HelloOk, got {other:?}"),
    }
}
