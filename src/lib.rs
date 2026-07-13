pub mod cli;
pub mod client;
pub mod commands;
pub mod display;
pub mod ipc;
pub mod name_gen;
pub mod paths;
pub mod server;
pub mod session;
pub mod term_guard;
pub mod text;
pub mod tui;

#[cfg(test)]
pub(crate) mod test_helpers;

use clap::Parser;

pub fn run() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    commands::dispatch(cli)
}
