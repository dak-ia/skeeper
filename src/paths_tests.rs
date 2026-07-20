use super::*;
use uuid::Uuid;

#[test]
fn xdg_takes_precedence() {
    let xdg = Path::new("/run/user/1000");
    let home = Path::new("/home/u");
    let got = runtime_dir_from(Some(xdg), Some(home)).unwrap();
    assert_eq!(got, Path::new("/run/user/1000/skeeper"));
}

#[test]
fn xdg_only_works() {
    let xdg = Path::new("/run/user/1000");
    let got = runtime_dir_from(Some(xdg), None).unwrap();
    assert_eq!(got, Path::new("/run/user/1000/skeeper"));
}

#[test]
fn home_fallback_when_no_xdg() {
    let home = Path::new("/home/u");
    let got = runtime_dir_from(None, Some(home)).unwrap();
    assert_eq!(got, Path::new("/home/u/.skeeper/run"));
}

#[test]
fn error_when_neither_available() {
    let err = runtime_dir_from(None, None).unwrap_err();
    assert!(err.to_string().contains("XDG_RUNTIME_DIR"));
}

#[test]
fn empty_xdg_falls_back_to_home() {
    let got = runtime_dir_from(Some(Path::new("")), Some(Path::new("/home/u"))).unwrap();
    assert_eq!(got, Path::new("/home/u/.skeeper/run"));
}

#[test]
fn both_empty_errors() {
    let err = runtime_dir_from(Some(Path::new("")), Some(Path::new(""))).unwrap_err();
    // Error message contains the env var name, not translated
    assert!(err.to_string().contains("XDG_RUNTIME_DIR"));
}

#[test]
fn meta_path_uses_uuid_hyphenated() {
    let dir = Path::new("/base");
    let id = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);
    let got = meta_path(dir, &id);
    assert_eq!(
        got,
        Path::new("/base/550e8400-e29b-41d4-a716-446655440000.json")
    );
}

#[test]
fn socket_path_uses_uuid_hyphenated() {
    let dir = Path::new("/base");
    let id = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);
    let got = socket_path(dir, &id);
    assert_eq!(
        got,
        Path::new("/base/550e8400-e29b-41d4-a716-446655440000.sock")
    );
}

#[test]
fn ctl_path_uses_uuid_hyphenated() {
    let dir = Path::new("/base");
    let id = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);
    let got = ctl_path(dir, &id);
    assert_eq!(
        got,
        Path::new("/base/550e8400-e29b-41d4-a716-446655440000.ctl")
    );
}

#[test]
fn ensure_runtime_dir_creates_with_0700() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("newly-created");
    ensure_runtime_dir(&target).unwrap();
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn ensure_runtime_dir_tightens_existing_0755_to_0700() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("existing-loose");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

    ensure_runtime_dir(&target).unwrap();
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}
