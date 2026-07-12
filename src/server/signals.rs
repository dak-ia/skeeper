use std::sync::atomic::Ordering;

use anyhow::{Result, anyhow};
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

use super::TERM_REQUESTED;

extern "C" fn signal_flag_handler(_: nix::libc::c_int) {
    TERM_REQUESTED.store(true, Ordering::SeqCst);
}

pub(super) fn install_termination_handlers() -> Result<()> {
    // SA_RESTARTでシグナル配送時にread/writeがEINTRで死なないようにする
    // (フラグはメインループがpollで検知する。EINTRで別スレッドが誤って落ちるのを防ぐ)
    let action = SigAction::new(
        SigHandler::Handler(signal_flag_handler),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    // signalハンドラのinstallはプロセス全体に効くので、testでは使わない前提
    unsafe { sigaction(Signal::SIGTERM, &action) }
        .map_err(|e| anyhow!("Failed to install SIGTERM handler: {e}"))?;
    unsafe { sigaction(Signal::SIGINT, &action) }
        .map_err(|e| anyhow!("Failed to install SIGINT handler: {e}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "signals_tests.rs"]
mod tests;
