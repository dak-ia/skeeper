use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::session::{self, SessionMeta};

/// runを抜けるときにソケット/メタファイルを掃除する。パニック時も動く。
#[allow(clippy::struct_field_names)]
pub(super) struct SessionFileGuard<'a> {
    pub(super) meta_path: &'a Path,
    pub(super) socket_path: &'a Path,
    pub(super) ctl_socket_path: &'a Path,
}

impl Drop for SessionFileGuard<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.ctl_socket_path);
        let _ = std::fs::remove_file(self.socket_path);
        let _ = std::fs::remove_file(self.meta_path);
    }
}

/// attach中に持っている共有状態(active_client + meta.attached_client_pid)を、
/// 早期return/パニックのどの経路でも一度だけ確実に解除するためのRAII guard。
pub(super) struct AttachStateGuard<'a> {
    pub(super) active_client: &'a Mutex<Option<Arc<Mutex<UnixStream>>>>,
    pub(super) meta: &'a Mutex<SessionMeta>,
    pub(super) meta_path: &'a Path,
    pub(super) armed: bool,
}

impl AttachStateGuard<'_> {
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AttachStateGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // pty_reader_loopがouter lockを保持したまま書き込む設計に依存して、
        // ここでNoneに戻せば「in-flight書き込みは完了→以降新規書き込みなし」となる
        *self.active_client.lock().unwrap() = None;
        let mut m = self.meta.lock().unwrap();
        m.attached_client_pid = None;
        let _ = session::write_meta_atomic(self.meta_path, &m);
    }
}

#[cfg(test)]
#[path = "guards_tests.rs"]
mod tests;
