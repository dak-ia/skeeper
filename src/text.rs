use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// 表示幅(East Asian Width考慮)ベースでpadまたは切り詰めする。
/// widthに収まらない場合は最後を`…`で置き換える(そのぶんwidthは-1ではなく-2まで先で詰める)
#[must_use]
pub fn pad_or_truncate_display(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let actual = UnicodeWidthStr::width(s);
    if actual <= width {
        let padding = width - actual;
        let mut out = String::with_capacity(s.len() + padding);
        out.push_str(s);
        for _ in 0..padding {
            out.push(' ');
        }
        return out;
    }
    // 切り詰め: 最後に`…`(1列)を入れるので、target = width - 1
    let target = width.saturating_sub(1);
    let mut out = String::new();
    let mut acc = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if acc + cw > target {
            break;
        }
        out.push(ch);
        acc += cw;
    }
    out.push('…');
    acc += 1;
    // 端数が余ったら空白でパディング(East Asian Widthの都合で1列不足するケース)
    for _ in acc..width {
        out.push(' ');
    }
    out
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
