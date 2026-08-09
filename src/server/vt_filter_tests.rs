use super::*;

#[test]
fn plain_text_passes_through_unchanged() {
    let input = b"hello world";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn strips_osc10_query_with_bel_terminator() {
    let input = b"\x1b]10;?\x07";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_osc11_query_with_st_terminator() {
    let input = b"\x1b]11;?\x1b\\";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_osc12_query() {
    let input = b"\x1b]12;?\x07";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn preserves_text_around_stripped_osc_query() {
    let input = b"before\x1b]10;?\x07after";
    assert_eq!(strip_terminal_queries(input), b"beforeafter");
}

#[test]
fn strips_multiple_consecutive_queries() {
    let input = b"\x1b]10;?\x07\x1b]11;?\x07\x1b]12;?\x07";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn preserves_osc10_set_only_strips_query() {
    // OSC 10 with actual color spec (not '?') = set command, keep it
    let input = b"\x1b]10;rgb:aaaa/bbbb/cccc\x07";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn preserves_osc_window_title() {
    let input = b"\x1b]0;my window title\x07";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn strips_primary_da_bare() {
    let input = b"\x1b[c";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_primary_da_with_zero_param() {
    let input = b"\x1b[0c";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_secondary_da() {
    let input = b"\x1b[>c";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_secondary_da_with_zero_param() {
    let input = b"\x1b[>0c";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_tertiary_da() {
    let input = b"\x1b[=c";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn preserves_unrelated_csi_erase_line() {
    let input = b"\x1b[K";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn preserves_unrelated_csi_cursor_show() {
    let input = b"\x1b[?25h";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn preserves_unrelated_csi_with_c_final_but_non_da_params() {
    // 例えばCSI 3 c(存在しないけどparam付きcとして扱う)はDA queryじゃないのでpass through
    let input = b"\x1b[3c";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn preserves_truncated_osc_at_end_of_input() {
    // 終端バイトがまだ来ていない → 触らない
    let input = b"before\x1b]10;?";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn preserves_truncated_csi_at_end_of_input() {
    let input = b"before\x1b[0";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn preserves_lone_escape() {
    let input = b"\x1b";
    assert_eq!(strip_terminal_queries(input), input);
}

#[test]
fn strips_query_between_normal_output() {
    let input = b"line1\r\n\x1b]11;?\x07line2\r\n\x1b[c$ ";
    assert_eq!(strip_terminal_queries(input), b"line1\r\nline2\r\n$ ");
}

// ---- bare ESC abort inside OSC (ECMA-48準拠) ----

#[test]
fn aborts_outer_osc_when_bare_esc_starts_new_sequence() {
    // 外側OSC 10;? が終端しないうちに内側OSC 11;? が始まるケース。
    // 外側は abort として drop、内側は正しく剥がされて全体が空になるべき
    let input = b"\x1b]10;?\x1b]11;?\x07";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn aborts_outer_osc_and_preserves_following_normal_text() {
    // 外側OSC 10;? が bare ESC で中断され、その ESC から始まる別の完結OSC(title)は残る
    let input = b"\x1b]10;?\x1b]0;title\x07after";
    assert_eq!(strip_terminal_queries(input), b"\x1b]0;title\x07after");
}

// ---- DSR (Device Status Report) 応答系 ----

#[test]
fn strips_dsr_status_report_query() {
    // CSI 5n → CSI 0n を返す
    let input = b"\x1b[5n";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_dsr_cursor_position_query() {
    // CSI 6n → CSI row;col R を返す
    let input = b"\x1b[6n";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_dsr_private_cursor_position_query() {
    // CSI ? 6 n → 応答系
    let input = b"\x1b[?6n";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn preserves_csi_mode_set_and_reset_final_h_l() {
    // n分岐追加後もh/lを絶対にstripしない防波堤
    let show_cursor = b"\x1b[?25h";
    assert_eq!(strip_terminal_queries(show_cursor), show_cursor);
    let hide_cursor = b"\x1b[?25l";
    assert_eq!(strip_terminal_queries(hide_cursor), hide_cursor);
    let bracketed_paste_on = b"\x1b[?2004h";
    assert_eq!(
        strip_terminal_queries(bracketed_paste_on),
        bracketed_paste_on
    );
    // ?6h/?6lはDECOM(origin mode) set/reset。?6nと同じ数字だが応答は返さないので絶対にstripしない
    let origin_mode_on = b"\x1b[?6h";
    assert_eq!(strip_terminal_queries(origin_mode_on), origin_mode_on);
    let origin_mode_off = b"\x1b[?6l";
    assert_eq!(strip_terminal_queries(origin_mode_off), origin_mode_off);
}

#[test]
fn preserves_csi_n_variant_that_is_not_dsr() {
    // CSI 3n (存在しないpsだがnを終端に持つ非DSRとしてpassthroughされるべき)
    let input = b"\x1b[3n";
    assert_eq!(strip_terminal_queries(input), input);
}

// ---- OSC 4;N;? (palette color query) ----

#[test]
fn strips_osc4_palette_color_query() {
    // OSC 4;0;? → 色番号0(black)の値を問い合わせ
    let input = b"\x1b]4;0;?\x07";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_osc4_palette_query_with_st_terminator() {
    let input = b"\x1b]4;15;?\x1b\\";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn strips_osc4_palette_query_multidigit() {
    let input = b"\x1b]4;255;?\x07";
    assert_eq!(strip_terminal_queries(input), b"");
}

#[test]
fn preserves_osc4_palette_set_only_strips_query() {
    // OSC 4;N;rgb:... は色設定コマンドで応答は返さない → 剥がしてはいけない
    let input = b"\x1b]4;0;rgb:0000/0000/0000\x07";
    assert_eq!(strip_terminal_queries(input), input);
}
