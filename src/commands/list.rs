use std::os::unix::net::UnixStream;
use std::path::Path;

use time::UtcOffset;
use uuid::Uuid;

use crate::cli::ListArgs;
use crate::ipc::{self, ControlMsg, ControlResponse};
use crate::session::SessionMeta;
use crate::text::pad_or_truncate_display;
use crate::{display, paths, session};

const ID_COL_WIDTH: usize = 8;
const NAME_COL_WIDTH: usize = 20;
const STATE_COL_WIDTH: usize = 13;
const DATE_COL_WIDTH: usize = 19;
const PROTO_COL_WIDTH: usize = 8;
const INDENT_WIDTH: usize = 10;
// Linuxのpid_max既定値(4194304)は7桁。SSH_CONNECTIONと違い切り詰めるとpid特定を損なうので必ず収まる幅にする
const PID_COL_WIDTH: usize = 7;
const TTY_COL_WIDTH: usize = 12;
const SSH_COL_WIDTH: usize = 24;

pub(crate) fn run(args: ListArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let mut sessions = session::list_all_meta(&base_dir).unwrap_or_default();

    if sessions.is_empty() {
        println!("No sessions");
        return Ok(());
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    let offset = display::local_offset();

    // (me)マーカーはsub-tableでしか使わないので、通常listでは不要なsocket問い合わせを省く。
    // session外(SKEEPER_SESSION_ID未設定)や接続失敗もcurrent_session_stdin_pid内でNoneに落ちる
    let current_pid = if args.detail {
        current_session_stdin_pid(&base_dir)
    } else {
        None
    };

    println!("{}", render_header());
    for s in &sessions {
        println!("{}", render_session_line(s, offset));
        if args.detail {
            for line in render_client_sub_table(s, current_pid, offset) {
                println!("{line}");
            }
        }
    }

    Ok(())
}

fn render_header() -> String {
    format!(
        "{id}  {name}  {state}  {created}  {last}  {proto}  CWD",
        id = pad_or_truncate_display("ID", ID_COL_WIDTH),
        name = pad_or_truncate_display("NAME", NAME_COL_WIDTH),
        state = pad_or_truncate_display("STATE", STATE_COL_WIDTH),
        created = pad_or_truncate_display("CREATED", DATE_COL_WIDTH),
        last = pad_or_truncate_display("LAST ATTACHED", DATE_COL_WIDTH),
        proto = pad_or_truncate_display("PROTO", PROTO_COL_WIDTH),
    )
}

fn render_session_line(s: &SessionMeta, offset: UtcOffset) -> String {
    let id_short = display::id_short(&s.id);
    let created = display::format_local(s.created_at, offset);
    let last_attached = s
        .last_attached_at
        .map_or_else(|| "-".to_string(), |t| display::format_local(t, offset));
    let state_label = state_label(s.attached_clients.len());
    let proto = proto_marker(s.ipc_protocol_version);
    let cwd = s.cwd.to_string_lossy();
    format!(
        "{id}  {name}  {state}  {created}  {last}  {proto}  {cwd}",
        id = pad_or_truncate_display(&id_short, ID_COL_WIDTH),
        name = pad_or_truncate_display(&s.name, NAME_COL_WIDTH),
        state = pad_or_truncate_display(&state_label, STATE_COL_WIDTH),
        created = pad_or_truncate_display(&created, DATE_COL_WIDTH),
        last = pad_or_truncate_display(&last_attached, DATE_COL_WIDTH),
        proto = pad_or_truncate_display(proto, PROTO_COL_WIDTH),
    )
}

fn render_client_sub_table(
    s: &SessionMeta,
    current_pid: Option<u32>,
    offset: UtcOffset,
) -> Vec<String> {
    if s.attached_clients.is_empty() {
        return Vec::new();
    }
    let indent = " ".repeat(INDENT_WIDTH);
    let mut lines = Vec::with_capacity(s.attached_clients.len() + 1);
    lines.push(format!(
        "{indent}{pid}  {tty}  {ssh}  ATTACHED",
        pid = pad_or_truncate_display("PID", PID_COL_WIDTH),
        tty = pad_or_truncate_display("TTY", TTY_COL_WIDTH),
        ssh = pad_or_truncate_display("SSH_CONNECTION", SSH_COL_WIDTH),
    ));
    for c in &s.attached_clients {
        let tty = c.tty.as_deref().unwrap_or("-");
        let ssh = c.ssh_connection.as_deref().unwrap_or("-");
        let attached = display::format_local(c.attached_at, offset);
        let me_suffix = if current_pid == Some(c.pid) {
            " (me)"
        } else {
            ""
        };
        lines.push(format!(
            "{indent}{pid}  {tty}  {ssh}  {attached}{me}",
            pid = pad_or_truncate_display(&c.pid.to_string(), PID_COL_WIDTH),
            tty = pad_or_truncate_display(tty, TTY_COL_WIDTH),
            ssh = pad_or_truncate_display(ssh, SSH_COL_WIDTH),
            me = me_suffix,
        ));
    }
    lines
}

fn state_label(n: usize) -> std::borrow::Cow<'static, str> {
    match n {
        0 => "detached".into(),
        1 => "attached".into(),
        _ => format!("attached ({n})").into(),
    }
}

fn proto_marker(version: u32) -> &'static str {
    if version < ipc::IPC_PROTOCOL_VERSION {
        "outdated"
    } else {
        "-"
    }
}

/// SKEEPER_SESSION_IDが設定されていれば、そのsessionのctlに問い合わせて直近stdin送信元のpidを取得する。
/// session外・接続失敗・pid=0はすべてNone(マーカーなし)として扱う
fn current_session_stdin_pid(base_dir: &Path) -> Option<u32> {
    let id_str = std::env::var("SKEEPER_SESSION_ID").ok()?;
    let id = Uuid::parse_str(&id_str).ok()?;
    let ctl_path = paths::ctl_path(base_dir, &id);
    let mut stream = UnixStream::connect(&ctl_path).ok()?;
    ipc::write_control_msg(&mut stream, &ControlMsg::QueryCurrentClient).ok()?;
    let ControlResponse::CurrentClient { pid } = ipc::read_control_response(&mut stream).ok()?
    else {
        return None;
    };
    // pid=0は「まだ誰もstdin送っていない」状態。マーカーを付ける対象がいない
    if pid == 0 { None } else { Some(pid) }
}

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;
