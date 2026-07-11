use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "skeeper",
    version,
    // Default -V for version, but we want lowercase -v, so override
    disable_version_flag = true,
    // Note: do NOT disable_help_flag globally — it propagates to subcommands
    // and breaks `skeeper list --help` etc. clap's default `-h/--help` in
    // English is fine for us.
    about = "Simple terminal session keeper",
    help_template = "\
{name} v{version}
{about}

{usage-heading} {usage}

{all-args}"
)]
// version フィールドは Action=Version トリガ用のダミー。non-exhaustive目的ではない
#[allow(clippy::manual_non_exhaustive)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Print version and exit
    #[arg(short = 'v', long, action = clap::ArgAction::Version)]
    version: (),
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new session and attach to it
    #[command(visible_alias = "n")]
    New(NewArgs),
    /// Attach to a session
    #[command(visible_alias = "a")]
    Attach(AttachArgs),
    /// List all sessions
    #[command(visible_alias = "ls")]
    List,
    /// Detach from the current session
    #[command(visible_alias = "d")]
    Detach,
    /// Rename a session
    #[command(visible_alias = "r")]
    Rename(RenameArgs),
    /// Kill (destroy) a session
    #[command(visible_alias = "k")]
    Kill(KillArgs),
    /// Prune orphan session files (server crashed or otherwise dead)
    #[command(visible_alias = "p")]
    Prune,
    /// (internal) Run as a session server
    #[command(name = "__server-run", hide = true)]
    ServerRun(ServerRunArgs),
}

#[derive(Debug, Args)]
pub struct NewArgs {
    /// Session name (a random name is generated if omitted)
    pub name: Option<String>,
    /// Create only, do not attach
    #[arg(short = 'd', long)]
    pub detached: bool,
    /// Shell to run inside the session
    #[arg(short = 's', long)]
    pub shell: Option<String>,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    /// Session name to attach to (opens an interactive picker if omitted)
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct RenameArgs {
    /// New session name
    pub new_name: String,
    /// Rename the session with this name (default: the current one)
    #[arg(short = 'o', long = "old")]
    pub old: Option<String>,
}

#[derive(Debug, Args)]
pub struct KillArgs {
    /// Session name to kill (default: the current one)
    pub name: Option<String>,
    /// Kill all sessions
    #[arg(short = 'a', long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ServerRunArgs {
    #[arg(long)]
    pub id: Uuid,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub cwd: PathBuf,
    #[arg(long)]
    pub shell: PathBuf,
}
