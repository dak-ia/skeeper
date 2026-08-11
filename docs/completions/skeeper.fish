# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_skeeper_global_optspecs
    string join \n v/version h/help
end

function __fish_skeeper_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_skeeper_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_skeeper_using_subcommand
    set -l cmd (__fish_skeeper_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c skeeper -n "__fish_skeeper_needs_command" -s v -l version -d 'Print version and exit'
complete -c skeeper -n "__fish_skeeper_needs_command" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "new" -d 'Create a new session and attach to it'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "n" -d 'Create a new session and attach to it'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "attach" -d 'Attach to a session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "a" -d 'Attach to a session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "list" -d 'List all sessions'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "ls" -d 'List all sessions'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "detach" -d 'Detach from the current session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "d" -d 'Detach from the current session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "rename" -d 'Rename a session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "r" -d 'Rename a session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "kill" -d 'Kill (destroy) a session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "k" -d 'Kill (destroy) a session'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "prune" -d 'Prune orphan session files (server crashed or otherwise dead)'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "p" -d 'Prune orphan session files (server crashed or otherwise dead)'
complete -c skeeper -n "__fish_skeeper_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c skeeper -n "__fish_skeeper_using_subcommand new" -s s -l shell -d 'Shell to run inside the session' -r
complete -c skeeper -n "__fish_skeeper_using_subcommand new" -s c -l cwd -d 'Initial working directory (default: current directory)' -r -F
complete -c skeeper -n "__fish_skeeper_using_subcommand new" -s d -l detached -d 'Create only, do not attach'
complete -c skeeper -n "__fish_skeeper_using_subcommand new" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand n" -s s -l shell -d 'Shell to run inside the session' -r
complete -c skeeper -n "__fish_skeeper_using_subcommand n" -s c -l cwd -d 'Initial working directory (default: current directory)' -r -F
complete -c skeeper -n "__fish_skeeper_using_subcommand n" -s d -l detached -d 'Create only, do not attach'
complete -c skeeper -n "__fish_skeeper_using_subcommand n" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand attach" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand a" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand list" -s d -l detail -d 'Show detail sub-table for each attached client (pid, tty, ssh, attach time)'
complete -c skeeper -n "__fish_skeeper_using_subcommand list" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand ls" -s d -l detail -d 'Show detail sub-table for each attached client (pid, tty, ssh, attach time)'
complete -c skeeper -n "__fish_skeeper_using_subcommand ls" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand detach" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand d" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand rename" -s o -l old -d 'Rename the session with this name (default: the current one)' -r
complete -c skeeper -n "__fish_skeeper_using_subcommand rename" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand r" -s o -l old -d 'Rename the session with this name (default: the current one)' -r
complete -c skeeper -n "__fish_skeeper_using_subcommand r" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand kill" -s a -l all -d 'Kill all sessions'
complete -c skeeper -n "__fish_skeeper_using_subcommand kill" -s y -l yes -d 'Skip confirmation prompt'
complete -c skeeper -n "__fish_skeeper_using_subcommand kill" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand k" -s a -l all -d 'Kill all sessions'
complete -c skeeper -n "__fish_skeeper_using_subcommand k" -s y -l yes -d 'Skip confirmation prompt'
complete -c skeeper -n "__fish_skeeper_using_subcommand k" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand prune" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand p" -s h -l help -d 'Print help'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "new" -d 'Create a new session and attach to it'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "attach" -d 'Attach to a session'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "list" -d 'List all sessions'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "detach" -d 'Detach from the current session'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "rename" -d 'Rename a session'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "kill" -d 'Kill (destroy) a session'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "prune" -d 'Prune orphan session files (server crashed or otherwise dead)'
complete -c skeeper -n "__fish_skeeper_using_subcommand help; and not __fish_seen_subcommand_from new attach list detach rename kill prune help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
