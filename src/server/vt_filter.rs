/// scrollback replayに乗せる前に、端末が応答すべきquery系escape sequenceを剥がす。
/// 新しいclient端末の再応答がshellの入力を汚染するのを防ぐための後処理。
/// 完結bufferを一括で渡す前提(pty chunkを跨ぐstreaming用途は非対応)
#[must_use]
pub fn strip_terminal_queries(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == 0x1b && i + 1 < input.len() {
            let next = input[i + 1];
            if next == b'['
                && let Some(end) = try_match_csi_query(input, i)
            {
                i = end;
                continue;
            }
            if next == b']' {
                match try_match_osc_query(input, i) {
                    OscScan::QueryMatched { end } | OscScan::Aborted { end } => {
                        i = end;
                        continue;
                    }
                    OscScan::NotQuery | OscScan::Partial => {}
                }
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

/// input[start]==ESC, input[start+1]==`[` を前提に、DA/DSRのquery形なら sequence終端の次のindexを返す。
/// intermediate byte(0x20-0x2f)を含むCSIはNone(現DA/DSRはintermediate無し)
fn try_match_csi_query(input: &[u8], start: usize) -> Option<usize> {
    // ECMA-48: CSI = ESC [ parameter bytes(0x30-0x3f) final byte(0x40-0x7e)
    let mut j = start + 2;
    let params_start = j;
    while j < input.len() && (0x30..=0x3f).contains(&input[j]) {
        j += 1;
    }
    if j >= input.len() {
        return None;
    }
    let final_byte = input[j];
    if !(0x40..=0x7e).contains(&final_byte) {
        return None;
    }
    let params = &input[params_start..j];
    let matched = match final_byte {
        // DA: Primary=""|"0" / Secondary=">"|">0" / Tertiary="="|"=0"
        b'c' => matches!(params, b"" | b"0" | b">" | b">0" | b"=" | b"=0"),
        // DSR: CSI 5n / CSI 6n / CSI ?6n(private cursor position report)。
        // h/l(mode set/reset)を絶対に巻き込まないよう`n` finalに厳密限定する
        b'n' => matches!(params, b"5" | b"6" | b"?6"),
        _ => false,
    };
    if matched { Some(j + 1) } else { None }
}

enum OscScan {
    /// 対象queryが完結。`[start..end)`を丸ごとdrop
    QueryMatched { end: usize },
    /// 途中で裸ESCが出てabort。`[start..end)`をdropし`end`(=次のESC)からscan再開
    Aborted { end: usize },
    /// 完結したが対象queryではない。呼び出し側は原文をそのまま残す
    NotQuery,
    /// 終端未到達(bufferがそこで切れている)。呼び出し側は原文をそのまま残す
    Partial,
}

/// input[start]==ESC, input[start+1]==`]` を前提にOSCをscan。対象queryなら剥がす。
/// abort時は外側scanが aborted position から再scanできるよう end を返す
fn try_match_osc_query(input: &[u8], start: usize) -> OscScan {
    let params_start = start + 2;
    let mut j = params_start;
    while j < input.len() {
        if input[j] == 0x07 {
            let params = &input[params_start..j];
            return if is_osc_query(params) {
                OscScan::QueryMatched { end: j + 1 }
            } else {
                OscScan::NotQuery
            };
        }
        if input[j] == 0x1b {
            if j + 1 >= input.len() {
                // ESCで終わっている → 途中まで
                return OscScan::Partial;
            }
            if input[j + 1] == b'\\' {
                // 正常なST(ESC \)
                let params = &input[params_start..j];
                return if is_osc_query(params) {
                    OscScan::QueryMatched { end: j + 2 }
                } else {
                    OscScan::NotQuery
                };
            }
            // 裸ESC → OSC abort。 [start..j) をdropし、j(=次のESC)から再scan
            return OscScan::Aborted { end: j };
        }
        j += 1;
    }
    OscScan::Partial
}

fn is_osc_query(params: &[u8]) -> bool {
    // 定番の色query
    if matches!(params, b"10;?" | b"11;?" | b"12;?") {
        return true;
    }
    // OSC 4;<index>;? のpalette query。indexは10進数字1桁以上
    if let Some(rest) = params.strip_prefix(b"4;")
        && let Some(idx) = rest.strip_suffix(b";?")
        && !idx.is_empty()
        && idx.iter().all(u8::is_ascii_digit)
    {
        return true;
    }
    false
}

#[cfg(test)]
#[path = "vt_filter_tests.rs"]
mod tests;
