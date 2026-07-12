use super::*;
use time::macros::datetime;

// 各フィールド(年/月/日/時/分/秒)を全部違う値にしてゼロ埋め・並び順のミスを検出しやすくする
const FIXTURE: time::OffsetDateTime = datetime!(2000-01-02 03:04:05 UTC);

#[test]
fn format_local_utc() {
    assert_eq!(format_local(FIXTURE, UtcOffset::UTC), "2000-01-02 03:04:05");
}

#[test]
fn format_local_with_positive_offset() {
    let offset = UtcOffset::from_hms(9, 0, 0).unwrap();
    assert_eq!(format_local(FIXTURE, offset), "2000-01-02 12:04:05");
}

#[test]
fn format_local_with_negative_offset_can_roll_back_to_prev_day() {
    // -5時間で日付が前日に繰り下がるケース
    let offset = UtcOffset::from_hms(-5, 0, 0).unwrap();
    assert_eq!(format_local(FIXTURE, offset), "2000-01-01 22:04:05");
}

#[test]
fn id_short_takes_first_8_chars() {
    let id = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);
    assert_eq!(id_short(&id), "550e8400");
}
