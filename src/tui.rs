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
    let state_label = if s.attached_client_pid.is_some() {
        "attached"
    } else {
        "detached"
    };
    let name_col = pad_or_truncate_display(&s.name, NAME_COL_WIDTH);
    let state_col = pad_or_truncate_display(state_label, STATE_COL_WIDTH);
    format!("{name_col}  {state_col}  {created}")
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
