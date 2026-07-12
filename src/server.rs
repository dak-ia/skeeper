use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use time::OffsetDateTime;

use crate::cli::ServerRunArgs;
use crate::paths;
use crate::session::{self, SessionMeta};

mod guards;
mod signals;

use guards::SessionFileGuard;
use signals::install_termination_handlers;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// SIGTERM/SIGINTを受けたときに立てるフラグ。メインループが検知して掃除経路へ入る
pub(super) static TERM_REQUESTED: AtomicBool = AtomicBool::new(false);

/// セッションサーバ本体。ソケットとmeta.jsonを用意して、SIGTERM/SIGINT/子プロセス終了までブロックする。
/// この時点の実装はclient受け入れをしないので、attachはPR#c(attach handler追加)まで機能しない
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

    // 同じUUIDで残っているstaleなソケットがあれば除去
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&ctl_socket_path);
    let _listener = UnixListener::bind(&socket_path)?;
    let _ctl_listener = UnixListener::bind(&ctl_socket_path)?;

    // HOMEフォールバック(~/.skeeper/runが0755など)でも他ユーザーからconnectできないよう、
    // ソケットのmodeを0600に絞る。XDG_RUNTIME_DIR(0700)配下では冗長だが実害なし
    let owner_only = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(&socket_path, owner_only.clone())?;
    std::fs::set_permissions(&ctl_socket_path, owner_only)?;

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

    // TERM要求が来るまで待つ。次PRでpty spawnとaccept loopを載せる
    while !TERM_REQUESTED.load(Ordering::Acquire) {
        thread::sleep(POLL_INTERVAL);
    }

    Ok(())
}
