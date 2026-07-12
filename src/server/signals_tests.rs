use super::*;
use std::sync::atomic::Ordering;

use crate::server::TERM_REQUESTED;

// TERM_REQUESTEDはprocess-globalなstaticなので、他テストと並列で読み書きが衝突する
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn run_sets_term_requested_flag() {
    let _guard = LOCK.lock().unwrap();

    TERM_REQUESTED.store(false, Ordering::SeqCst);
    signal_flag_handler(0);
    assert!(TERM_REQUESTED.load(Ordering::SeqCst));

    // 他テストへの影響を残さないよう後始末
    TERM_REQUESTED.store(false, Ordering::SeqCst);
}
