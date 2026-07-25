use std::os::unix::net::UnixStream;

use anyhow::Context;

use crate::cli::RenameArgs;
use crate::ipc::{self, ControlMsg, ControlResponse, RenameResponse};
use crate::{paths, session};

use super::current_session_id;

pub(crate) fn run(args: RenameArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let sessions = session::list_all_meta(&base_dir).unwrap_or_default();

    // uniqueness判定はサーバ側でflock下に行うため、ここでは事前チェックしない
    let target = if let Some(old_name) = args.old {
        sessions
            .iter()
            .find(|m| m.name == old_name)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("Session '{old_name}' not found. Run `skeeper ls` to see the list")
            })?
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
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Current session metadata is missing (server may have crashed). Exit this shell or run `skeeper prune`"
                )
            })?
    };

    let ctl_path = paths::ctl_path(&base_dir, &target.id);
    let mut stream = UnixStream::connect(&ctl_path).with_context(|| {
        format!(
            "Cannot reach the server for session '{}'; it may have crashed. Run `skeeper prune`. (socket: {})",
            target.name,
            ctl_path.display()
        )
    })?;
    ipc::write_control_msg(
        &mut stream,
        &ControlMsg::RequestRename {
            new_name: args.new_name.clone(),
        },
    )?;
    let response = ipc::read_control_response(&mut stream)
        .context("Failed to read rename response from server")?;
    match response {
        ControlResponse::Rename(RenameResponse::Ok) => {
            println!("Renamed '{}' to '{}'", target.name, args.new_name);
            Ok(())
        }
        ControlResponse::Rename(RenameResponse::Unchanged) => {
            println!("Session name unchanged: '{}'", args.new_name);
            Ok(())
        }
        ControlResponse::Rename(RenameResponse::Conflict) => {
            anyhow::bail!("Session name '{}' is already in use", args.new_name)
        }
        ControlResponse::Rename(RenameResponse::Failed) => {
            anyhow::bail!("Server failed to rename session (try again in a moment)")
        }
        // 具体的なvariantはユーザーには意味不明なので隠す
        _ => anyhow::bail!(
            "Server returned an unexpected response (client and server versions may differ)"
        ),
    }
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
