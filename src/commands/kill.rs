use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;

use crate::cli::KillArgs;
use crate::session::SessionMeta;
use crate::{paths, session, tui};

use super::current_session_id;

const KILL_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const KILL_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn run(args: KillArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let sessions = session::list_all_meta(&base_dir).unwrap_or_default();
    let skip_confirm = args.yes;

    // 対象を決定する:
    //   1) -a all             → 全セッション、y/N確認
    //   2) name指定           → その1件、確認不要
    //   3) 引数なし+セッション内 → 現セッション、y/N確認
    //   4) 引数なし+セッション外 → TUIで複数選択、選択自体が確認代わり
    let (targets, requires_confirmation) = if args.all {
        if sessions.is_empty() {
            println!("No sessions to kill");
            return Ok(());
        }
        (sessions.clone(), true)
    } else if let Some(name) = args.name {
        let t = sessions
            .iter()
            .find(|m| m.name == name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Session '{name}' not found"))?;
        (vec![t], false)
    } else if let Some(id) = current_session_id(&base_dir) {
        let t = sessions
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Current session metadata not found"))?;
        (vec![t], true)
    } else {
        if sessions.is_empty() {
            anyhow::bail!("No sessions to kill");
        }
        let Some(indices) = tui::pick_sessions_multi(&sessions)? else {
            return Ok(()); // Esc/qでキャンセル
        };
        let selected: Vec<SessionMeta> = indices.into_iter().map(|i| sessions[i].clone()).collect();
        (selected, false)
    };

    if requires_confirmation && !skip_confirm {
        let names: Vec<&str> = targets.iter().map(|m| m.name.as_str()).collect();
        let prompt = if targets.len() == 1 {
            format!("Kill session '{}'? [y/N] ", names[0])
        } else {
            format!(
                "Kill {} sessions: {}? [y/N] ",
                targets.len(),
                names.join(", ")
            )
        };
        if !confirm(&prompt)? {
            println!("Aborted");
            return Ok(());
        }
    }

    for t in &targets {
        kill_one_session(&base_dir, t)?;
    }
    let names: Vec<&str> = targets.iter().map(|m| m.name.as_str()).collect();
    println!(
        "Killed {} session{}: {}",
        targets.len(),
        if targets.len() == 1 { "" } else { "s" },
        names.join(", ")
    );
    Ok(())
}

fn kill_one_session(base_dir: &Path, meta: &SessionMeta) -> anyhow::Result<()> {
    let sock = paths::socket_path(base_dir, &meta.id);
    let ctl = paths::ctl_path(base_dir, &meta.id);
    let meta_path = paths::meta_path(base_dir, &meta.id);

    // pid==0はkill(2)で自プロセスグループ全体への配送になり、is_orphanの結果に関わらず
    // 親シェルまで巻き添えにするので絶対にsignalを送らない。
    // is_orphanは非対応OS(macOS等)ではErr(bail)を返すのでunwrap_or(false)で通常経路に流す
    let orphan_or_zero = meta.server_pid == 0 || session::is_orphan(meta).unwrap_or(false);
    if orphan_or_zero {
        let _ = std::fs::remove_file(&ctl);
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_file(&meta_path);
        return Ok(());
    }

    let pid = i32::try_from(meta.server_pid)
        .map_err(|_| anyhow::anyhow!("Invalid server pid {}", meta.server_pid))?;

    match signal::kill(Pid::from_raw(pid), Signal::SIGTERM) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
        Err(e) => return Err(e.into()),
    }

    let start = Instant::now();
    while start.elapsed() < KILL_CLEANUP_TIMEOUT {
        if !sock.exists() && !meta_path.exists() {
            // サーバのSessionFileGuardがctlを先に消してから他を消すので、
            // sock/metaが消えている時点でctlも消えているはず。念のため明示除去
            let _ = std::fs::remove_file(&ctl);
            return Ok(());
        }
        std::thread::sleep(KILL_POLL_INTERVAL);
    }

    // SIGKILLする前にもう一度is_orphanを確認する。3秒の間にサーバプロセスが死んで
    // PIDが別プロセスに再利用された可能性があるため
    if session::is_orphan(meta).unwrap_or(false) {
        let _ = std::fs::remove_file(&ctl);
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_file(&meta_path);
        return Ok(());
    }

    let _ = signal::kill(Pid::from_raw(pid), Signal::SIGKILL);
    let _ = std::fs::remove_file(&ctl);
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&meta_path);
    Ok(())
}

/// 対話プロンプト。y/yes(大小関係なし)ならtrue、その他はfalse。stdin無しの場合もfalse
fn confirm(prompt: &str) -> anyhow::Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut line = String::new();
    let stdin = io::stdin();
    let read = stdin.lock().read_line(&mut line)?;
    if read == 0 {
        return Ok(false);
    }
    let trimmed = line.trim().to_ascii_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
}

#[cfg(test)]
#[path = "kill_tests.rs"]
mod tests;
