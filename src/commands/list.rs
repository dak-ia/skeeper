use crate::text::pad_or_truncate_display;
use crate::{display, paths, session};

pub(crate) fn run() -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let mut sessions = session::list_all_meta(&base_dir).unwrap_or_default();

    if sessions.is_empty() {
        println!("No sessions");
        return Ok(());
    }

    // 新しい順(作成日時降順)
    sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));

    // ローカルoffsetを取れる環境ではローカル時刻で表示する
    let offset = display::local_offset();

    // ヘッダ。cwdは可変長にしたいので最後、他の列は固定幅
    println!(
        "{id}  {name}  {state}  {created}  {last}  CWD",
        id = pad_or_truncate_display("ID", 8),
        name = pad_or_truncate_display("NAME", 20),
        state = pad_or_truncate_display("STATE", 8),
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
            state = pad_or_truncate_display(&state_label, 8),
            created = pad_or_truncate_display(&created, 19),
            last = pad_or_truncate_display(&last_attached, 19),
        );
    }

    Ok(())
}

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;
