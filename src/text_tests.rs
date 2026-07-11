use super::*;

#[test]
fn pad_ascii_short() {
    assert_eq!(pad_or_truncate_display("abc", 5), "abc  ");
}

#[test]
fn pad_ascii_exact() {
    assert_eq!(pad_or_truncate_display("abcde", 5), "abcde");
}

#[test]
fn truncate_ascii_long() {
    assert_eq!(pad_or_truncate_display("abcdefghij", 5), "abcd…");
}

#[test]
fn pad_japanese_short() {
    // "あい" = 4列(2文字×East Asian Wide)なので、幅6なら2列分space
    assert_eq!(pad_or_truncate_display("あい", 6), "あい  ");
}

#[test]
fn truncate_japanese_long() {
    // "あいうえお" = 10列。幅6なら"あい"(4列) + "…"(1列) + " "(1列) = 6列
    let got = pad_or_truncate_display("あいうえお", 6);
    assert_eq!(UnicodeWidthStr::width(got.as_str()), 6);
    assert!(got.contains('…'));
}

#[test]
fn empty_string_pads_to_full_width() {
    assert_eq!(pad_or_truncate_display("", 4), "    ");
}

#[test]
fn zero_width_returns_empty() {
    assert_eq!(pad_or_truncate_display("abc", 0), String::new());
}
