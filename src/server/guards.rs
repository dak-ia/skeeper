use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::session::{self, SessionMeta};

use super::ClientHandle;

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

/// 各clientのattach中に持っている共有状態(active_clients登録 + meta.attached_client_pids)を、
/// 早期return/パニックのどの経路でも一度だけ確実に解除するためのRAII guard。
pub(super) struct AttachStateGuard<'a> {
    pub(super) client_pid: u32,
    pub(super) active_clients: &'a Mutex<HashMap<u32, ClientHandle>>,
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
        // pty_reader_loopがactive_clientsのlockを保持したまま書き込む設計に依存して、
        // ここでmapから外せば「in-flight書き込みは完了→以降新規書き込みなし」となる
        {
            let mut acl = self.active_clients.lock().unwrap();
            acl.remove(&self.client_pid);
        }
        let mut m = self.meta.lock().unwrap();
        m.attached_client_pids.retain(|&p| p != self.client_pid);
        let _ = session::write_meta_atomic(self.meta_path, &m);
    }
}

#[cfg(test)]
#[path = "guards_tests.rs"]
mod tests;
