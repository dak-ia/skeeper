use super::*;

fn parse(argv: &[&str]) -> Cli {
    Cli::try_parse_from(argv).expect("parse failed")
}

#[test]
fn new_without_args_has_no_name_and_no_flags() {
    let cli = parse(&["skeeper", "new"]);
    match cli.command {
        Some(Command::New(args)) => {
            assert_eq!(args.name, None);
            assert!(!args.detached);
            assert_eq!(args.shell, None);
        }
        other => panic!("expected New, got {other:?}"),
    }
}

#[test]
fn new_with_name_detached_and_shell_captures_all() {
    let cli = parse(&["skeeper", "new", "-d", "--shell", "/bin/bash", "myname"]);
    match cli.command {
        Some(Command::New(args)) => {
            assert_eq!(args.name.as_deref(), Some("myname"));
            assert!(args.detached);
            assert_eq!(args.shell.as_deref(), Some("/bin/bash"));
            assert_eq!(args.cwd, None);
        }
        other => panic!("expected New, got {other:?}"),
    }
}

#[test]
fn new_with_cwd_long_flag_captures_path() {
    let cli = parse(&["skeeper", "new", "--cwd", "/tmp"]);
    match cli.command {
        Some(Command::New(args)) => {
            assert_eq!(args.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
        }
        other => panic!("expected New, got {other:?}"),
    }
}

#[test]
fn new_with_cwd_short_flag_captures_path() {
    let cli = parse(&["skeeper", "new", "-c", "/tmp"]);
    match cli.command {
        Some(Command::New(args)) => {
            assert_eq!(args.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
        }
        other => panic!("expected New, got {other:?}"),
    }
}

#[test]
fn attach_with_name_sets_target() {
    let cli = parse(&["skeeper", "attach", "foo"]);
    match cli.command {
        Some(Command::Attach(args)) => assert_eq!(args.name.as_deref(), Some("foo")),
        other => panic!("expected Attach, got {other:?}"),
    }
}

#[test]
fn attach_without_name_leaves_name_none() {
    let cli = parse(&["skeeper", "attach"]);
    match cli.command {
        Some(Command::Attach(args)) => assert_eq!(args.name, None),
        other => panic!("expected Attach, got {other:?}"),
    }
}

#[test]
fn list_and_ls_alias_both_resolve_to_list() {
    assert!(matches!(
        parse(&["skeeper", "list"]).command,
        Some(Command::List(_))
    ));
    assert!(matches!(
        parse(&["skeeper", "ls"]).command,
        Some(Command::List(_))
    ));
}

#[test]
fn list_with_long_flag() {
    let cli = parse(&["skeeper", "list", "--long"]);
    match cli.command {
        Some(Command::List(args)) => assert!(args.long),
        other => panic!("expected List, got {other:?}"),
    }
}

#[test]
fn list_short_l_flag() {
    let cli = parse(&["skeeper", "list", "-l"]);
    match cli.command {
        Some(Command::List(args)) => assert!(args.long),
        other => panic!("expected List, got {other:?}"),
    }
}

#[test]
fn rename_with_only_positional_treats_it_as_new_name() {
    let cli = parse(&["skeeper", "rename", "newname"]);
    match cli.command {
        Some(Command::Rename(args)) => {
            assert_eq!(args.new_name, "newname");
            assert_eq!(args.old, None);
        }
        other => panic!("expected Rename, got {other:?}"),
    }
}

#[test]
fn rename_with_old_flag_targets_named_session() {
    let cli = parse(&["skeeper", "rename", "-o", "oldname", "newname"]);
    match cli.command {
        Some(Command::Rename(args)) => {
            assert_eq!(args.new_name, "newname");
            assert_eq!(args.old.as_deref(), Some("oldname"));
        }
        other => panic!("expected Rename, got {other:?}"),
    }
}

#[test]
fn kill_without_args_has_no_target_and_no_all_flag() {
    let cli = parse(&["skeeper", "kill"]);
    match cli.command {
        Some(Command::Kill(args)) => {
            assert_eq!(args.name, None);
            assert!(!args.all);
            assert!(!args.yes);
        }
        other => panic!("expected Kill, got {other:?}"),
    }
}

#[test]
fn kill_with_positional_targets_named_session() {
    let cli = parse(&["skeeper", "kill", "foo"]);
    match cli.command {
        Some(Command::Kill(args)) => {
            assert_eq!(args.name.as_deref(), Some("foo"));
            assert!(!args.all);
            assert!(!args.yes);
        }
        other => panic!("expected Kill, got {other:?}"),
    }
}

#[test]
fn kill_with_all_flag_sets_all_true() {
    let cli = parse(&["skeeper", "kill", "-a"]);
    match cli.command {
        Some(Command::Kill(args)) => {
            assert_eq!(args.name, None);
            assert!(args.all);
            assert!(!args.yes);
        }
        other => panic!("expected Kill, got {other:?}"),
    }
}

#[test]
fn kill_with_yes_short_flag() {
    let cli = parse(&["skeeper", "kill", "-y"]);
    match cli.command {
        Some(Command::Kill(args)) => {
            assert!(args.yes);
            assert!(!args.all);
        }
        other => panic!("expected Kill, got {other:?}"),
    }
}

#[test]
fn kill_with_yes_long_flag_and_name() {
    let cli = parse(&["skeeper", "kill", "--yes", "foo"]);
    match cli.command {
        Some(Command::Kill(args)) => {
            assert_eq!(args.name.as_deref(), Some("foo"));
            assert!(args.yes);
        }
        other => panic!("expected Kill, got {other:?}"),
    }
}

#[test]
fn kill_with_all_and_yes_combined() {
    let cli = parse(&["skeeper", "kill", "-a", "-y"]);
    match cli.command {
        Some(Command::Kill(args)) => {
            assert!(args.all);
            assert!(args.yes);
        }
        other => panic!("expected Kill, got {other:?}"),
    }
}

#[test]
fn d_alias_resolves_to_detach() {
    assert!(matches!(
        parse(&["skeeper", "d"]).command,
        Some(Command::Detach)
    ));
}

#[test]
fn p_alias_resolves_to_prune() {
    assert!(matches!(
        parse(&["skeeper", "p"]).command,
        Some(Command::Prune)
    ));
}
