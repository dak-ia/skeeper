use std::os::unix::net::UnixStream;

use anyhow::Context;

use crate::cli::AttachArgs;
use crate::ipc::{self, ControlMsg};
use crate::{client, paths, session, tui};

use super::current_session_id;

pub(crate) fn run(args: AttachArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;

    // session内は手作業`d`→`a`をouter client向けswitch指示1回に集約する
    if let Some(current_id) = current_session_id(&base_dir) {
        let Some(target_name) = args.name.as_deref() else {
            anyhow::bail!(
                "Session name is required when running `skeeper attach` inside a session"
            );
        };
        let sessions = session::list_all_meta(&base_dir).unwrap_or_default();
        let target = sessions
            .iter()
            .find(|m| m.name == target_name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Session '{target_name}' not found. Run `skeeper ls` to see the list"
                )
            })?;
        if target.id == current_id {
            println!("Already attached to session '{target_name}'");
            return Ok(());
        }
        let current_ctl = paths::ctl_path(&base_dir, &current_id);
        let target_socket = paths::socket_path(&base_dir, &target.id);
        let mut stream = UnixStream::connect(&current_ctl).with_context(|| {
            format!(
                "Failed to connect to control socket {}",
                current_ctl.display()
            )
        })?;
        ipc::write_control_msg(
            &mut stream,
            &ControlMsg::SwitchClient {
                target_socket_path: target_socket,
            },
        )?;
        // 内側processはここでexit。outer clientがSwitchTo受信で乗り換える
        return Ok(());
    }

    let sessions = session::list_all_meta(&base_dir).unwrap_or_default();

    let target = if let Some(name) = args.name {
        sessions
            .into_iter()
            .find(|m| m.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!("Session '{name}' not found. Run `skeeper ls` to see the list")
            })?
    } else {
        if sessions.is_empty() {
            anyhow::bail!("No sessions. Run `skeeper new` to create one");
        }
        let Some(idx) = tui::pick_session(&sessions)? else {
            return Ok(()); // ユーザーがEsc/qでキャンセル
        };
        sessions
            .into_iter()
            .nth(idx)
            .expect("selected index is valid")
    };

    let socket_path = paths::socket_path(&base_dir, &target.id);
    client::attach(&socket_path)
}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
