use std::os::unix::net::UnixStream;

use anyhow::Context;

use crate::ipc::{self, ControlMsg};
use crate::{paths, session};

use super::current_session_id;

pub(crate) fn run() -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let Some(session_id) = current_session_id(&base_dir) else {
        anyhow::bail!("Must be run inside a skeeper session");
    };

    // 接続中でないときに送るとサーバ側でフラグだけ残って次回attach即detachになる。
    // クライアント側でも防御しておく(サーバ側でもattach開始時にリセットする)
    let meta_path = paths::meta_path(&base_dir, &session_id);
    let meta = session::read_meta(&meta_path).context("Failed to read session metadata")?;
    if meta.attached_client_pid.is_none() {
        anyhow::bail!("No client is currently attached");
    }

    let ctl_path = paths::ctl_path(&base_dir, &session_id);
    let mut stream = UnixStream::connect(&ctl_path)
        .with_context(|| format!("Failed to connect to control socket {}", ctl_path.display()))?;
    ipc::write_control_msg(&mut stream, &ControlMsg::RequestDetach)?;
    // fire-and-forget: サーバがattached_loopで検知して既存のDetach経路を通す
    Ok(())
}

#[cfg(test)]
#[path = "detach_tests.rs"]
mod tests;
