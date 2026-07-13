use std::path::Path;

use uuid::Uuid;

use crate::cli::{Cli, Command};
use crate::paths;

mod attach;
mod bare;
mod detach;
mod kill;
mod list;
mod new;
mod prune;
mod rename;

pub fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        None => bare::run(),
        Some(Command::New(args)) => new::run(args),
        Some(Command::Attach(args)) => attach::run(args),
        Some(Command::List) => list::run(),
        Some(Command::Detach) => detach::run(),
        Some(Command::Rename(args)) => rename::run(args),
        Some(Command::Kill(args)) => kill::run(args),
        Some(Command::Prune) => prune::run(),
    }
}

/// 現在のシェルがskeeperセッション内で動いているかを、二段構えで判定する
/// (1) `SKEEPER_SESSION_ID` env varが立っていて、かつ有効なUUIDにparseできる
/// (2) そのIDのmeta.jsonがruntime_dirに実在する
///
/// bashrc誤exportは(2)で外れて未接続扱いになる。
/// なお、サーバがSIGKILL等で強制終了した後にmeta.jsonが取り残されるケースは、
/// 現状ここでは検出できず「接続中」と誤判定する。この掃除はskeeper pruneの責務
pub(crate) fn current_session_id(base_dir: &Path) -> Option<Uuid> {
    let id_str = std::env::var("SKEEPER_SESSION_ID").ok()?;
    let id = Uuid::parse_str(&id_str).ok()?;
    if paths::meta_path(base_dir, &id).exists() {
        Some(id)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
