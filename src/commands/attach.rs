use crate::cli::AttachArgs;
use crate::{client, paths, session, tui};

use super::current_session_id;

pub(crate) fn run(args: AttachArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;

    if current_session_id(&base_dir).is_some() {
        anyhow::bail!("Already attached to a session. Run `skeeper d` to detach first");
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
