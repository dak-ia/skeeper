use std::os::unix::net::UnixStream;

use anyhow::Context;

use crate::cli::RenameArgs;
use crate::ipc::{self, ControlMsg};
use crate::{paths, session};

use super::current_session_id;

pub(crate) fn run(args: RenameArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let sessions = session::list_all_meta(&base_dir).unwrap_or_default();

    // 対象セッションを決める: -o指定ならその名前、無指定なら接続中のセッション
    let target = if let Some(old_name) = args.old {
        sessions
            .iter()
            .find(|m| m.name == old_name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Session '{old_name}' not found"))?
    } else {
        let Some(id) = current_session_id(&base_dir) else {
            anyhow::bail!(
                "Must be run inside a skeeper session (or use -o to specify a session name)"
            );
        };
        sessions
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Current session metadata not found"))?
    };

    // 新しい名前が他のセッションと衝突しないか。自分と同じ名前(no-op)は許容
    if target.name != args.new_name && sessions.iter().any(|m| m.name == args.new_name) {
        anyhow::bail!("Session name '{}' is already in use", args.new_name);
    }

    // 制御ソケット経由でサーバに依頼(サーバ側でmeta更新+atomic write)
    let ctl_path = paths::ctl_path(&base_dir, &target.id);
    let mut stream = UnixStream::connect(&ctl_path)
        .with_context(|| format!("Failed to connect to control socket {}", ctl_path.display()))?;
    ipc::write_control_msg(
        &mut stream,
        &ControlMsg::RequestRename {
            new_name: args.new_name.clone(),
        },
    )?;
    println!("Renamed '{}' to '{}'", target.name, args.new_name);
    Ok(())
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
