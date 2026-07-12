use super::*;

// TTY必須なenter()はテスト環境で失敗するため、Dropだけを検証する
// disable_raw_mode / LeaveAlternateScreenは非TTY環境でも失敗を握り潰す前提

#[test]
fn drop_does_not_panic_on_non_tty() {
    let g = TerminalGuard;
    drop(g);
}

#[test]
fn drop_is_idempotent_across_multiple_instances() {
    // 連続してdropしても状態が壊れずpanicしないこと
    drop(TerminalGuard);
    drop(TerminalGuard);
    drop(TerminalGuard);
}
