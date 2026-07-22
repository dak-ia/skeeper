use std::os::unix::net::UnixStream;
use std::path::Path;

use uuid::Uuid;

use crate::cli::ListArgs;
use crate::ipc::{self, ControlMsg, ControlResponse};
use crate::text::pad_or_truncate_display;
use crate::{display, paths, session};

const STATE_COL_WIDTH: usize = 13;

pub(crate) fn run(args: ListArgs) -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let mut sessions = session::list_all_meta(&base_dir).unwrap_or_default();

    if sessions.is_empty() {
        println!("No sessions");
        return Ok(());
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    let offset = display::local_offset();

    // pty内から呼ばれたときだけ、自分のsessionのserverに「今stdin送ってる自分」を問い合わせる。
    // session外からのlistでは意味を持たないのでスキップ(問い合わせ先が特定できない)
    let current_pid = current_session_stdin_pid(&base_dir);

    // ヘッダ。cwdは可変長にしたいので最後、他の列は固定幅
    println!(
        "{id}  {name}  {state}  {created}  {last}  CWD",
        id = pad_or_truncate_display("ID", 8),
        name = pad_or_truncate_display("NAME", 20),
        state = pad_or_truncate_display("STATE", STATE_COL_WIDTH),
        created = pad_or_truncate_display("CREATED", 19),
        last = pad_or_truncate_display("LAST ATTACHED", 19),
    );

    for s in sessions {
        let id_short = display::id_short(&s.id);
        let created = display::format_local(s.created_at, offset);
        let last_attached = s
            .last_attached_at
            .map_or_else(|| "-".to_string(), |t| display::format_local(t, offset));
        let n = s.attached_client_pids.len();
        // 0=detached, 1=attached, 2以上=attached (N)。formatは複数client時のみ
        let state_label: std::borrow::Cow<'_, str> = match n {
            0 => "detached".into(),
            1 => "attached".into(),
            _ => format!("attached ({n})").into(),
        };
        let cwd = s.cwd.to_string_lossy();

        println!(
            "{id}  {name}  {state}  {created}  {last}  {cwd}",
            id = id_short,
            name = pad_or_truncate_display(&s.name, 20),
            state = pad_or_truncate_display(&state_label, STATE_COL_WIDTH),
            created = pad_or_truncate_display(&created, 19),
            last = pad_or_truncate_display(&last_attached, 19),
        );

        // --long指定時のみ、attach中clientのpidを補助行として出す(未接続なら追加行なし)
        if args.long && !s.attached_client_pids.is_empty() {
            println!(
                "  Clients: {}",
                format_clients(&s.attached_client_pids, current_pid)
            );
        }
    }

    Ok(())
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

/// "1001, 1002 (me), 1003" のような表示文字列を組み立てる。currentがNoneならマーカーなし
fn format_clients(pids: &[u32], current: Option<u32>) -> String {
    let parts: Vec<String> = pids
        .iter()
        .map(|&p| {
            if current == Some(p) {
                format!("{p} (me)")
            } else {
                p.to_string()
            }
        })
        .collect();
    parts.join(", ")
}

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;
