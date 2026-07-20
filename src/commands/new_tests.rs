use super::*;

#[test]
fn resolve_shell_arg_wins() {
    assert_eq!(
        resolve_shell(Some("/bin/zsh"), Some("/bin/bash")),
        PathBuf::from("/bin/zsh")
    );
}

#[test]
fn resolve_shell_falls_back_to_env() {
    assert_eq!(
        resolve_shell(None, Some("/bin/bash")),
        PathBuf::from("/bin/bash")
    );
}

#[test]
fn resolve_shell_defaults_when_both_missing() {
    assert_eq!(resolve_shell(None, None), PathBuf::from("/bin/sh"));
}

#[test]
fn resolve_shell_empty_arg_treated_as_missing() {
    assert_eq!(
        resolve_shell(Some(""), Some("/bin/bash")),
        PathBuf::from("/bin/bash")
    );
}

#[test]
fn resolve_shell_empty_env_treated_as_missing() {
    assert_eq!(resolve_shell(None, Some("")), PathBuf::from("/bin/sh"));
}

#[test]
fn resolve_cwd_none_uses_current() {
    let current = Path::new("/tmp");
    assert_eq!(resolve_cwd(None, current).unwrap(), PathBuf::from("/tmp"));
}

#[test]
fn resolve_cwd_absolute_arg_wins() {
    // /tmpは大抵存在するが、cross-platform配慮でexistsチェック付き
    let current = Path::new("/nonexistent-base-for-test");
    let result = resolve_cwd(Some(Path::new("/tmp")), current).unwrap();
    assert_eq!(result, PathBuf::from("/tmp"));
}

#[test]
fn resolve_cwd_relative_arg_absolutized() {
    let tmp = tempfile::tempdir().unwrap();
    let sub = tmp.path().join("subdir");
    std::fs::create_dir(&sub).unwrap();
    // 相対path "subdir" を tmp.path() 起点で解決すると sub になる
    let result = resolve_cwd(Some(Path::new("subdir")), tmp.path()).unwrap();
    assert_eq!(result, sub);
}

#[test]
fn resolve_cwd_nonexistent_path_errors() {
    let current = Path::new("/tmp");
    let err = resolve_cwd(Some(Path::new("/no/such/dir/skeeper-test")), current)
        .expect_err("expected error for nonexistent path");
    assert!(err.to_string().contains("--cwd"));
}

#[test]
fn resolve_cwd_file_not_directory_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("afile");
    std::fs::write(&file, b"content").unwrap();
    let err = resolve_cwd(Some(&file), Path::new("/tmp"))
        .expect_err("expected error for non-directory path");
    assert!(err.to_string().contains("not a directory"));
}
