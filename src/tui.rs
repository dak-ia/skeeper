use time::UtcOffset;

use crate::display;
use crate::session::SessionMeta;
use crate::text::pad_or_truncate_display;

const NAME_COL_WIDTH: usize = 24;
const STATE_COL_WIDTH: usize = 8;

mod multi;
mod single;

pub use multi::pick_sessions_multi;
pub use single::pick_session;

/// セッション1件を1行分のテキストとして整形する。
/// pick_session / pick_sessions_multiで共通利用
pub(super) fn session_line_text(s: &SessionMeta, offset: UtcOffset) -> String {
    let created = display::format_local(s.created_at, offset);
    let n = s.attached_client_pids.len();
    // 0=detached, 1=attached, 2以上=attached (N)。formatは複数client時のみ
    let state_label: std::borrow::Cow<'_, str> = match n {
        0 => "detached".into(),
        1 => "attached".into(),
        _ => format!("attached ({n})").into(),
    };
    let name_col = pad_or_truncate_display(&s.name, NAME_COL_WIDTH);
    let state_col = pad_or_truncate_display(&state_label, STATE_COL_WIDTH);
    format!("{name_col}  {state_col}  {created}")
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
