use std::collections::VecDeque;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use time::OffsetDateTime;

use crate::cli::ServerRunArgs;
use crate::paths;
use crate::session::{self, SessionMeta};

mod attach;
mod control;
mod guards;
mod pty_io;
mod signals;

use attach::handle_client;
use control::control_listener_loop;
use guards::SessionFileGuard;
use pty_io::pty_reader_loop;
use signals::install_termination_handlers;

pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const PTY_BUF_SIZE: usize = 4096;
/// 停滞クライアント検出用。ここを超えるとwriteがtimeoutで抜ける
pub(super) const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// handshake中に応答しないクライアントを切るため
pub(super) const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(5);
/// スクロールバックとして保持するpty出力のバイト数上限(≒数画面ぶん)
pub(super) const SCROLLBACK_MAX_BYTES: usize = 128 * 1024;

/// リングバッファ形式のスクロールバック。pty出力を捕えて、attach時に再生する
pub(super) type Scrollback = Arc<Mutex<VecDeque<u8>>>;

/// SIGTERM/SIGINTを受けたときに立てるフラグ。メインループが検知して掃除経路へ入る
pub(super) static TERM_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 制御ソケット経由でRequestDetachを受けたら立てるフラグ。attached_loopが検知して
/// 通常のdetach経路に合流する。single-session server前提のstatic
pub(super) static DETACH_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
pub(super) enum HandleOutcome {
    /// クライアントが自発的にdetachした。次のattach待ちに戻る
    Detached,
    /// クライアントとの接続が意図せず切れた。次のattach待ちに戻る
    Disconnected,
    /// クライアントが不正/衝突で拒否された。次のattach待ちに戻る
    Rejected,
    /// ptyの子プロセスが終了した / SIGTERMが要求された。サーバも終了する
    ChildExited,
}

/// セッションサーバ本体。pty起動 + 制御ソケット + pty出力→scrollback + client accept + attach handlerまでの全機能
#[allow(clippy::too_many_lines)] // 起動シーケンスの直列記述を優先
pub fn run(args: ServerRunArgs) -> Result<()> {
    // 親プロセスのセッションから切り離す(端末のCtrl+C等が伝播しないように)
    // 既にsession leaderの場合はEPERMになるが実害なし
    let _ = nix::unistd::setsid();

    // SIGTERM/SIGINTを受けたら掃除経路を通って落ちるようにハンドラを入れる
    install_termination_handlers()?;

    let ServerRunArgs {
        id,
        name,
        cwd,
        shell,
    } = args;

    let base_dir = paths::runtime_dir()?;
    std::fs::create_dir_all(&base_dir)?;
    let meta_path = paths::meta_path(&base_dir, &id);
    let socket_path = paths::socket_path(&base_dir, &id);
    let ctl_socket_path = paths::ctl_path(&base_dir, &id);

    // 以降のどの経路で抜けてもファイル掃除は自動で走る
    let _guard = SessionFileGuard {
        meta_path: &meta_path,
        socket_path: &socket_path,
        ctl_socket_path: &ctl_socket_path,
    };

    // サーバ起動時にstaticフラグを初期化(前回のプロセスからの影響を避ける、defensive)
    TERM_REQUESTED.store(false, Ordering::Release);
    DETACH_REQUESTED.store(false, Ordering::Release);

    // 同じUUIDで残っているstaleなソケットがあれば除去
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&ctl_socket_path);
    let listener = UnixListener::bind(&socket_path)?;
    listener.set_nonblocking(true)?;
    let ctl_listener = UnixListener::bind(&ctl_socket_path)?;

    // HOMEフォールバック(~/.skeeper/runが0755など)でも他ユーザーからconnectできないよう、
    // ソケットのmodeを0600に絞る。XDG_RUNTIME_DIR(0700)配下では冗長だが実害なし
    let owner_only = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(&socket_path, owner_only.clone())?;
    std::fs::set_permissions(&ctl_socket_path, owner_only)?;

    // pty
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| anyhow!("Failed to open pty: {e}"))?;

    let mut cmd = CommandBuilder::new(&shell);
    cmd.cwd(&cwd);
    cmd.env("SKEEPER_SESSION_ID", id.to_string());
    let child = pty_pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| anyhow!("Failed to start shell: {e}"))?;
    // サーバ側はslaveを保持しない(子プロセスだけがslaveを持てば十分)
    drop(pty_pair.slave);

    // 自プロセスのstart timeを取得(孤児判定用)
    let self_pid = std::process::id();
    let self_started_at = session::process_start_time(self_pid)?
        .ok_or_else(|| anyhow!("Failed to get own process start time"))?;

    // メタ初期化
    let meta_initial = SessionMeta {
        id,
        name,
        cwd,
        shell,
        created_at: OffsetDateTime::now_utc(),
        last_attached_at: None,
        server_pid: self_pid,
        server_started_at: self_started_at,
        attached_client_pid: None,
    };
    session::write_meta_atomic(&meta_path, &meta_initial)?;

    // reader/writer取り出し + masterはMutexで包む(mainスレッド内でresize/lockに使うだけ、Arc不要)
    let master_reader = pty_pair
        .master
        .try_clone_reader()
        .map_err(|e| anyhow!("Failed to clone pty reader: {e}"))?;
    let master_writer = pty_pair
        .master
        .take_writer()
        .map_err(|e| anyhow!("Failed to take pty writer: {e}"))?;
    let master: Mutex<Box<dyn MasterPty + Send>> = Mutex::new(pty_pair.master);

    // 共有状態
    let active_client: Arc<Mutex<Option<Arc<Mutex<UnixStream>>>>> = Arc::new(Mutex::new(None));
    // metaはcontrol_listener_loop(別スレッド)からもrename処理で書き換えるためArc
    let meta: Arc<Mutex<SessionMeta>> = Arc::new(Mutex::new(meta_initial));
    let child_exited = Arc::new(AtomicBool::new(false));
    let child_status: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
    // スクロールバック: pty出力の直近ぶんを保持し、新規attach時に再生する
    let scrollback: Scrollback =
        Arc::new(Mutex::new(VecDeque::with_capacity(SCROLLBACK_MAX_BYTES)));

    // 制御ソケットの受付スレッド(meta/meta_pathを共有)
    {
        let meta = Arc::clone(&meta);
        let meta_path_owned = meta_path.clone();
        thread::spawn(move || control_listener_loop(&ctl_listener, &meta, &meta_path_owned));
    }

    // 常時実行のバックグラウンドスレッド: ptyのstdoutを読んで、接続中のクライアントへ流す
    {
        let active_client = Arc::clone(&active_client);
        let scrollback = Arc::clone(&scrollback);
        thread::spawn(move || pty_reader_loop(master_reader, active_client, scrollback));
    }

    // 常時実行のバックグラウンドスレッド: 子プロセス終了を監視して exit code を保存
    {
        let child_exited = Arc::clone(&child_exited);
        let child_status = Arc::clone(&child_status);
        thread::spawn(move || {
            let mut child = child;
            let exit_code = child
                .wait()
                .ok()
                .and_then(|s| i32::try_from(s.exit_code()).ok());
            *child_status.lock().unwrap() = exit_code;
            child_exited.store(true, Ordering::Release);
        });
    }

    // メインの受付ループ
    let mut writer = master_writer;
    loop {
        if child_exited.load(Ordering::Acquire) || TERM_REQUESTED.load(Ordering::Acquire) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let outcome = handle_client(
                    stream,
                    &master,
                    &mut writer,
                    &active_client,
                    &scrollback,
                    &meta,
                    &meta_path,
                    &child_exited,
                    &child_status,
                );
                match outcome {
                    Ok(HandleOutcome::ChildExited) => break,
                    Ok(_) => (),
                    Err(e) => eprintln!("client handling error: {e:#}"),
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                eprintln!("accept error: {e:#}");
                break;
            }
        }
    }

    Ok(())
}
