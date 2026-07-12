use time::OffsetDateTime;
use time::UtcOffset;
use time::macros::format_description;
use uuid::Uuid;

/// ローカルoffsetを取得。取れなければUTC
#[must_use]
pub fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

/// OffsetDateTimeを"YYYY-MM-DD HH:MM:SS"形式に整形する。失敗時は"?"
#[must_use]
pub fn format_local(dt: OffsetDateTime, offset: UtcOffset) -> String {
    let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    dt.to_offset(offset)
        .format(&fmt)
        .unwrap_or_else(|_| "?".to_string())
}

/// UUIDの先頭8文字を返す
#[must_use]
pub fn id_short(id: &Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
