use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::paths;
use crate::session::{self, SessionMeta};

/// server::runの引数一式。CLI arg parsingとは切り離してある(fork子で組み立てて渡す)
#[derive(Debug)]
pub struct ServerRunArgs {
    pub id: Uuid,
    pub name: String,
    pub cwd: PathBuf,
    pub shell: PathBuf,
}

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

/// fanout queueの目安行数(SKEEPER_ATTACH_BUFFER_LINESで上書き可、100byte/行換算をPTY_BUF_SIZEメッセージ数に丸める)
pub(super) const DEFAULT_ATTACH_BUFFER_LINES: usize = 10000;
const APPROX_BYTES_PER_LINE: usize = 100;

/// SKEEPER_ATTACH_BUFFER_LINESからper-client sync_channelのslot数を算出(invalidはdefault、最低1)
pub(super) fn attach_buffer_capacity() -> usize {
    let lines = std::env::var("SKEEPER_ATTACH_BUFFER_LINES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_ATTACH_BUFFER_LINES);
    (lines * APPROX_BYTES_PER_LINE / PTY_BUF_SIZE).max(1)
}

/// リングバッファ形式のスクロールバック。pty出力を捕えて、attach時に再生する
pub(super) type Scrollback = Arc<Mutex<VecDeque<u8>>>;

/// SIGTERM/SIGINTを受けたときに立てるフラグ。メインループが検知して掃除経路へ入る
pub(super) static TERM_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 直近stdinを送ったclientのpid。control socket経由のRequestDetachで対象を決めるために使う。
/// 0は「まだ誰もstdin送っていない」を意味する。single-session server前提のstatic
pub(super) static LAST_STDIN_CLIENT: AtomicU32 = AtomicU32::new(0);

/// 各attachを一意に識別するcounter。同一pidが再attachしてきた際、古い側の後片付けが
/// 「今のslot所有者」を上書きしてしまわないようownership比較に使う。0は無効値
pub(super) static ATTACH_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// attached_loopに配送するイベント。client読み取りとpty出力を1つのchannelで受けて、
/// pty_reader_loopが個別clientのwriteをブロックしないようにする
pub(super) enum ClientEvent {
    /// clientから届いたメッセージ(またはread失敗)
    ClientMsg(io::Result<crate::ipc::ClientMsg>),
    /// pty_reader_loopが読み取ったptyのchunk。Arcで包んで全client向けに共有する
    PtyChunk(Arc<Vec<u8>>),
    /// control listenerが受けた `SwitchClient` を、対象clientのattached_loopに配送する。
    /// attached_loopがServerMsg::SwitchToを流して自身のloopを閉じる
    SwitchToRequested(PathBuf),
}

/// 接続中の各clientについて、pty出力を積むevent送信口と個別detachシグナルを持つ。
/// streamはattached_loopが自前で保持しているのでここには入れない
pub(super) struct ClientHandle {
    /// このattachを一意に識別するid。同一pidの新旧attachが混在した際にどちらの
    /// 後片付けが今のslot所有者かを判定するために使う
    pub(super) attach_id: u64,
    /// このclientの端末サイズ。multi-client時にmin集約でpty sizeを決める
    pub(super) cols: u16,
    pub(super) rows: u16,
    pub(super) should_detach: Arc<AtomicBool>,
    /// 停滞client検出のためboundedにする。fullでのtry_send失敗を「slow client」と扱い切断経路へ
    pub(super) event_tx: mpsc::SyncSender<ClientEvent>,
}

/// active_clients全体の最小(cols, rows)を求める。0 clientならNone
pub(super) fn aggregate_min_size(acl: &HashMap<u32, ClientHandle>) -> Option<(u16, u16)> {
    let mut cols = u16::MAX;
    let mut rows = u16::MAX;
    let mut any = false;
    for h in acl.values() {
        cols = cols.min(h.cols);
        rows = rows.min(h.rows);
        any = true;
    }
    if any { Some((cols, rows)) } else { None }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum HandleOutcome {
    /// クライアントが自発的にdetachした
    Detached,
    /// クライアントとの接続が意図せず切れた
    Disconnected,
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
    paths::ensure_runtime_dir(&base_dir)?;
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
    LAST_STDIN_CLIENT.store(0, Ordering::Release);

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

    let meta_initial = SessionMeta {
        id,
        name,
        cwd,
        shell,
        created_at: OffsetDateTime::now_utc(),
        last_attached_at: None,
        server_pid: self_pid,
        server_started_at: self_started_at,
        attached_client_pids: Vec::new(),
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
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pty_pair.master));

    let active_clients: Arc<Mutex<HashMap<u32, ClientHandle>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // metaはcontrol_listener_loop(別スレッド)からもrename処理で書き換えるためArc
    let meta: Arc<Mutex<SessionMeta>> = Arc::new(Mutex::new(meta_initial));
    let child_exited = Arc::new(AtomicBool::new(false));
    let child_status: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
    let scrollback: Scrollback =
        Arc::new(Mutex::new(VecDeque::with_capacity(SCROLLBACK_MAX_BYTES)));
    // writerも複数client threadから同時書き込みされるためArc<Mutex<>>で共有する
    let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(master_writer));

    // 制御ソケットの受付スレッド(active_clients/meta/meta_pathを共有)
    {
        let active_clients = Arc::clone(&active_clients);
        let meta = Arc::clone(&meta);
        let meta_path_owned = meta_path.clone();
        thread::spawn(move || {
            control_listener_loop(&ctl_listener, &active_clients, &meta, &meta_path_owned);
        });
    }

    // 常時実行のバックグラウンドスレッド: ptyのstdoutを読んで、接続中の全クライアントへfanout
    {
        let active_clients = Arc::clone(&active_clients);
        let scrollback = Arc::clone(&scrollback);
        thread::spawn(move || pty_reader_loop(master_reader, active_clients, scrollback));
    }

    // 常時実行のバックグラウンドスレッド: 子プロセス終了を監視してexit codeを保存
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

    // メインの受付ループ(1接続=1thread)
    loop {
        if child_exited.load(Ordering::Acquire) || TERM_REQUESTED.load(Ordering::Acquire) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let master = Arc::clone(&master);
                let writer = Arc::clone(&writer);
                let active_clients = Arc::clone(&active_clients);
                let scrollback = Arc::clone(&scrollback);
                let meta = Arc::clone(&meta);
                let meta_path = meta_path.clone();
                let child_exited = Arc::clone(&child_exited);
                let child_status = Arc::clone(&child_status);
                thread::spawn(move || {
                    match handle_client(
                        stream,
                        &master,
                        &writer,
                        &active_clients,
                        &scrollback,
                        &meta,
                        &meta_path,
                        &child_exited,
                        &child_status,
                    ) {
                        Ok(_) => {}
                        Err(e) => eprintln!("client handling error: {e:#}"),
                    }
                });
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

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
