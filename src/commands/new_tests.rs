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
