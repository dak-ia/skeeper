use std::io::stdout;

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// raw mode + alternate screenへの切替をRAIIで管理する。dropで確実に元に戻す
pub struct TerminalGuard;

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("Failed to enter raw mode")?;
        // raw modeを入れた後にalternate screen。失敗したらraw modeも戻す
        if let Err(e) = execute!(stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(e).context("Failed to enter alternate screen");
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // 逆順で戻す: alternate screen → raw mode
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
#[path = "term_guard_tests.rs"]
mod tests;
