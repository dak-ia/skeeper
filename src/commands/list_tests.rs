use super::*;

#[test]
fn run_returns_ok_when_runtime_dir_is_empty() {
    let _guard = crate::test_helpers::env_lock();
    let dir = tempfile::tempdir().unwrap();
    // HOMEフォールバックにも同じtempを向ける: XDG側が無視される変更が入ってもテスト隔離を保つ
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }
    run(ListArgs { long: false }).unwrap();
}
