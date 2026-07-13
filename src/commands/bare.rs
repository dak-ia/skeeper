use std::path::Path;

use anyhow::Context;
use clap::CommandFactory;
use uuid::Uuid;

use crate::cli::Cli;
use crate::{display, paths, session};

use super::current_session_id;

/// 引数なしで起動されたとき: 接続中なら現在のセッションを1行表示、そうでなければhelp
pub(crate) fn run() -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    if let Some(id) = current_session_id(&base_dir) {
        status(&base_dir, id)
    } else {
        print_help()
    }
}

fn status(base_dir: &Path, id: Uuid) -> anyhow::Result<()> {
    let meta_path = paths::meta_path(base_dir, &id);
    let meta = session::read_meta(&meta_path).context("Failed to read session metadata")?;

    let offset = display::local_offset();
    let created = display::format_local(meta.created_at, offset);
    let id_short = display::id_short(&meta.id);

    println!(
        "Session: {name} (id: {id_short}, created: {created})",
        name = meta.name,
    );
    Ok(())
}

fn print_help() -> anyhow::Result<()> {
    Cli::command().print_help()?;
    println!();
    Ok(())
}

#[cfg(test)]
#[path = "bare_tests.rs"]
mod tests;
